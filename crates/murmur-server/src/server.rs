use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use murmur_core::protocol::{
    ClientMessage, FramingError, Hello, LayoutCommand, PROTOCOL_VERSION, PaneAgentSnapshot,
    PaneTerminalFrame, PaneTerminalSnapshot, RuntimeEpoch, ServerId, ServerMessage,
    SessionBootstrap, SessionEvent, SessionId, TerminalFrameBatch, VersionCheck,
    WorkspaceGitSnapshot, check_version,
};
use murmur_core::{
    AgentSnapshot, GitRepository, PaneId, Session, TerminalAgentProbe, TerminalCommand,
    TerminalRuntime, TerminalSize, TerminalUpdate, TerminalView, WorkspaceId, create_worktree,
    default_worktree_root, discover_repository, open_worktree, remove_worktree,
    validate_worktree_removal,
};

use crate::client_writer::{ClientWriteItem, ClientWriter};
use crate::endpoint::{Endpoint, EndpointListener, EndpointStream, default_socket_path};

const ACCEPT_POLL: Duration = Duration::from_millis(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(4);
const EVENT_HISTORY_LIMIT: usize = 256;
const AGENT_SCAN_INTERVAL: Duration = Duration::from_millis(500);
const GIT_SCAN_INTERVAL: Duration = Duration::from_secs(2);
const TERMINAL_FRAME_INTERVAL: Duration = Duration::from_micros(16_667);

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub endpoint: Endpoint,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            endpoint: Endpoint::local(default_socket_path()),
        }
    }
}

#[derive(Clone)]
pub struct ServerHandle {
    stop: Arc<AtomicBool>,
    lifecycle: Arc<ServerLifecycle>,
    state: Arc<Mutex<RuntimeState>>,
}

pub struct ClientConnection {
    stream: EndpointStream,
    bootstrap: SessionBootstrap,
}

impl ClientConnection {
    pub fn connect(endpoint: &Endpoint, client_name: impl Into<String>) -> io::Result<Self> {
        Self::handshake(endpoint.connect()?, client_name)
    }

    fn handshake(mut stream: EndpointStream, client_name: impl Into<String>) -> io::Result<Self> {
        stream.set_handshake_timeout(Some(HANDSHAKE_TIMEOUT))?;
        murmur_core::protocol::write_message(
            &mut stream,
            &ClientMessage::Hello(Hello {
                version: PROTOCOL_VERSION,
                client_name: client_name.into(),
            }),
        )
        .map_err(|error| io::Error::other(error.to_string()))?;
        let welcome: ServerMessage = murmur_core::protocol::read_message(&mut stream)
            .map_err(|error| io::Error::other(error.to_string()))?;
        match welcome {
            ServerMessage::Welcome { error: None, .. } => {}
            ServerMessage::Welcome {
                error: Some(error), ..
            } => return Err(io::Error::new(io::ErrorKind::InvalidData, error)),
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unexpected welcome response: {other:?}"),
                ));
            }
        }
        let bootstrap = match murmur_core::protocol::read_message(&mut stream)
            .map_err(|error| io::Error::other(error.to_string()))?
        {
            ServerMessage::Bootstrap(bootstrap) => bootstrap,
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unexpected bootstrap response: {other:?}"),
                ));
            }
        };
        let _ = stream.set_handshake_timeout(None);
        Ok(Self { stream, bootstrap })
    }

    pub fn bootstrap(&self) -> &SessionBootstrap {
        &self.bootstrap
    }

    pub fn into_stream(self) -> EndpointStream {
        self.stream
    }
}

impl ServerHandle {
    pub fn stop(&self) {
        self.lifecycle.begin_stop();
        self.stop.store(true, Ordering::Release);
    }

    pub fn snapshot(&self) -> murmur_core::SessionSnapshot {
        self.state
            .lock()
            .expect("server state lock poisoned")
            .session
            .snapshot()
    }

    pub fn server_id(&self) -> ServerId {
        self.state
            .lock()
            .expect("server state lock poisoned")
            .server_id
    }

    pub fn runtime_epoch(&self) -> RuntimeEpoch {
        self.state
            .lock()
            .expect("server state lock poisoned")
            .runtime_epoch
    }
}

pub struct BoundServer {
    endpoint: Endpoint,
    listener: EndpointListener,
    stop: Arc<AtomicBool>,
    lifecycle: Arc<ServerLifecycle>,
    state: Arc<Mutex<RuntimeState>>,
    next_client_id: Arc<AtomicU64>,
}

impl BoundServer {
    pub fn bind(config: ServerConfig) -> io::Result<Self> {
        let listener = config.endpoint.bind()?;
        listener.set_nonblocking(true)?;
        let state = RuntimeState::new(&config.endpoint);
        Ok(Self {
            endpoint: config.endpoint,
            listener,
            stop: Arc::new(AtomicBool::new(false)),
            lifecycle: Arc::new(ServerLifecycle::default()),
            state: Arc::new(Mutex::new(state)),
            next_client_id: Arc::new(AtomicU64::new(1)),
        })
    }

    pub fn handle(&self) -> ServerHandle {
        ServerHandle {
            stop: Arc::clone(&self.stop),
            lifecycle: Arc::clone(&self.lifecycle),
            state: Arc::clone(&self.state),
        }
    }

    pub fn local_addr(&self) -> io::Result<Option<std::net::SocketAddr>> {
        self.listener.local_addr()
    }

    pub fn run(self) -> io::Result<()> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_time()
            .build()
            .map_err(|error| {
                io::Error::other(format!("failed to create server runtime: {error}"))
            })?;
        runtime.block_on(async move {
            tokio::task::spawn_blocking(move || self.run_blocking())
                .await
                .map_err(|error| io::Error::other(format!("server task failed: {error}")))?
        })
    }

    fn run_blocking(self) -> io::Result<()> {
        while !self.stop.load(Ordering::Acquire) {
            match self.listener.accept() {
                Ok(stream) => {
                    let client_id = self.next_client_id.fetch_add(1, Ordering::Relaxed);
                    let state = Arc::clone(&self.state);
                    let stop = Arc::clone(&self.stop);
                    let lifecycle = Arc::clone(&self.lifecycle);
                    thread::spawn(move || handle_client(stream, client_id, state, stop, lifecycle));
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(ACCEPT_POLL);
                }
                Err(error) => {
                    self.lifecycle.begin_stop();
                    self.stop.store(true, Ordering::Release);
                    self.lifecycle.wait_for_external_operations();
                    self.state
                        .lock()
                        .expect("server state lock poisoned")
                        .terminals
                        .clear();
                    let _ = self.endpoint.cleanup();
                    return Err(error);
                }
            }
        }
        self.lifecycle.wait_for_external_operations();
        self.state
            .lock()
            .expect("server state lock poisoned")
            .terminals
            .clear();
        self.endpoint.cleanup()
    }
}

#[derive(Default)]
struct ServerLifecycle {
    state: Mutex<ServerLifecycleState>,
    idle: Condvar,
}

#[derive(Default)]
struct ServerLifecycleState {
    stopping: bool,
    external_operations: usize,
}

impl ServerLifecycle {
    fn begin_external(self: &Arc<Self>) -> Option<ExternalOperationGuard> {
        let mut state = self.state.lock().expect("server lifecycle lock poisoned");
        if state.stopping {
            return None;
        }
        state.external_operations += 1;
        Some(ExternalOperationGuard {
            lifecycle: Arc::clone(self),
        })
    }

    fn begin_stop(&self) {
        self.state
            .lock()
            .expect("server lifecycle lock poisoned")
            .stopping = true;
    }

    fn is_stopping(&self) -> bool {
        self.state
            .lock()
            .expect("server lifecycle lock poisoned")
            .stopping
    }

    fn wait_for_external_operations(&self) {
        let state = self.state.lock().expect("server lifecycle lock poisoned");
        let _state = self
            .idle
            .wait_while(state, |state| state.external_operations != 0)
            .expect("server lifecycle lock poisoned");
    }
}

struct ExternalOperationGuard {
    lifecycle: Arc<ServerLifecycle>,
}

impl Drop for ExternalOperationGuard {
    fn drop(&mut self) {
        let mut state = self
            .lifecycle
            .state
            .lock()
            .expect("server lifecycle lock poisoned");
        state.external_operations -= 1;
        if state.external_operations == 0 {
            self.lifecycle.idle.notify_all();
        }
    }
}

struct RuntimeState {
    // ponytail: one Session behind one lock for MVP; split the registry/locks when concurrency requires it.
    server_id: ServerId,
    runtime_epoch: RuntimeEpoch,
    session_id: SessionId,
    sequence: u64,
    session: Session,
    terminals: std::collections::HashMap<PaneId, TerminalRuntime>,
    exited_terminals: std::collections::HashSet<PaneId>,
    agents: std::collections::HashMap<PaneId, AgentSnapshot>,
    workspace_git: std::collections::HashMap<WorkspaceId, GitRepository>,
    workspace_git_scanned_at: std::collections::HashMap<WorkspaceId, Instant>,
    worktree_root: Option<PathBuf>,
    active_controller: Option<u64>,
    events: std::collections::VecDeque<SequencedEvent>,
    subscribers: std::collections::HashMap<u64, ClientSubscriber>,
}

struct ClientSubscriber {
    writer: ClientWriter,
    terminal_baselines: std::collections::HashMap<PaneId, TerminalView>,
    pending_terminals: std::collections::HashSet<PaneId>,
}

impl ClientSubscriber {
    fn try_send_terminal_render(
        &mut self,
        data: Vec<u8>,
        baselines: Vec<(PaneId, TerminalView)>,
    ) -> Result<(), mpsc::TrySendError<Vec<u8>>> {
        self.writer.try_send_render(data)?;
        for (pane_id, view) in baselines {
            self.terminal_baselines.insert(pane_id, view);
        }
        Ok(())
    }
}

struct SequencedEvent {
    sequence: u64,
    event: SessionEvent,
}

impl RuntimeState {
    fn new(endpoint: &Endpoint) -> Self {
        Self {
            server_id: ServerId(stable_endpoint_id(endpoint)),
            runtime_epoch: RuntimeEpoch(runtime_epoch()),
            session_id: SessionId(1),
            sequence: 0,
            session: Session::new(),
            terminals: std::collections::HashMap::new(),
            exited_terminals: std::collections::HashSet::new(),
            agents: std::collections::HashMap::new(),
            workspace_git: std::collections::HashMap::new(),
            workspace_git_scanned_at: std::collections::HashMap::new(),
            worktree_root: default_worktree_root().ok(),
            active_controller: None,
            events: std::collections::VecDeque::new(),
            subscribers: std::collections::HashMap::new(),
        }
    }

    fn bootstrap(&self) -> SessionBootstrap {
        let mut terminals = Vec::new();
        for workspace in self.session.workspaces() {
            for tab in workspace.tabs() {
                for pane in tab.panes() {
                    if let Some(runtime) = self.terminals.get(&pane.id()) {
                        terminals.push(PaneTerminalSnapshot {
                            pane_id: pane.id(),
                            view: runtime.view(),
                            exited: self.exited_terminals.contains(&pane.id()),
                        });
                    }
                }
            }
        }
        SessionBootstrap {
            server_id: self.server_id,
            runtime_epoch: self.runtime_epoch,
            session_id: self.session_id,
            sequence: self.sequence,
            snapshot: self.session.snapshot(),
            terminals,
            agents: self
                .agents
                .iter()
                .map(|(&pane_id, &agent)| PaneAgentSnapshot { pane_id, agent })
                .collect(),
            workspace_git: self
                .workspace_git
                .iter()
                .map(|(&workspace_id, repository)| workspace_git_snapshot(workspace_id, repository))
                .collect(),
            zoomed_panes: self
                .session
                .workspaces()
                .iter()
                .flat_map(|workspace| workspace.tabs())
                .filter_map(|tab| tab.zoomed_pane_id())
                .collect(),
        }
    }

    fn publish_layout_change(&mut self, origin_client_id: u64, origin: &ClientWriter) -> bool {
        let event = SessionEvent::LayoutChanged;
        self.publish_event(event, Some((origin_client_id, origin)))
    }

    fn publish_background(&mut self, event: SessionEvent) {
        self.publish_event(event, None);
    }

    fn publish_event(&mut self, event: SessionEvent, origin: Option<(u64, &ClientWriter)>) -> bool {
        self.events.push_back(SequencedEvent {
            sequence: self.sequence.saturating_add(1),
            event: event.clone(),
        });
        self.sequence = self.sequence.saturating_add(1);
        while self.events.len() > EVENT_HISTORY_LIMIT {
            self.events.pop_front();
        }
        let message = ServerMessage::Event {
            server_id: self.server_id,
            session_id: self.session_id,
            sequence: self.sequence,
            event,
        };
        let data = frame_message(&message).expect("Session event must fit a protocol frame");
        let origin_failed =
            origin.is_some_and(|(_, writer)| writer.send_reliable(data.clone()).is_err());
        self.subscribers.retain(|client_id, subscriber| {
            if origin.is_some_and(|(origin_client_id, _)| *client_id == origin_client_id) {
                !origin_failed
            } else {
                subscriber.writer.send_reliable(data.clone()).is_ok()
            }
        });
        origin_failed
    }

    fn publish_terminal(&mut self, pane_id: PaneId, view: &TerminalView) {
        let client_ids = self.subscribers.keys().copied().collect::<Vec<_>>();
        for client_id in client_ids {
            if let Some(subscriber) = self.subscribers.get_mut(&client_id) {
                subscriber.pending_terminals.insert(pane_id);
            }
            self.flush_terminal_render(client_id, Some((pane_id, view)));
        }
    }

    fn flush_terminal_render(&mut self, client_id: u64, latest: Option<(PaneId, &TerminalView)>) {
        let Some(subscriber) = self.subscribers.get(&client_id) else {
            return;
        };
        let pending = subscriber
            .pending_terminals
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let prepared = pending
            .iter()
            .filter_map(|pane_id| {
                let current = latest
                    .filter(|(latest_pane_id, _)| latest_pane_id == pane_id)
                    .map(|(_, view)| view.clone())
                    .or_else(|| self.terminals.get(pane_id).map(TerminalRuntime::view))?;
                let frame =
                    TerminalView::frame_from(subscriber.terminal_baselines.get(pane_id), &current)?;
                Some((*pane_id, current, frame))
            })
            .collect::<Vec<_>>();

        if prepared.is_empty() {
            if let Some(subscriber) = self.subscribers.get_mut(&client_id) {
                subscriber.pending_terminals.clear();
                subscriber
                    .terminal_baselines
                    .retain(|pane_id, _| self.terminals.contains_key(pane_id));
            }
            return;
        }

        let mut frames = Vec::with_capacity(prepared.len());
        let mut baselines = Vec::with_capacity(prepared.len());
        for (pane_id, current, frame) in prepared {
            frames.push(PaneTerminalFrame { pane_id, frame });
            baselines.push((pane_id, current));
        }
        let Ok(data) = frame_terminal_batches(self.server_id, self.session_id, frames) else {
            return;
        };
        let result = self
            .subscribers
            .get_mut(&client_id)
            .expect("subscriber exists while preparing a terminal render")
            .try_send_terminal_render(data, baselines);
        match result {
            Ok(()) => {
                let subscriber = self
                    .subscribers
                    .get_mut(&client_id)
                    .expect("subscriber exists while committing a terminal render");
                for pane_id in pending {
                    subscriber.pending_terminals.remove(&pane_id);
                }
            }
            Err(mpsc::TrySendError::Full(_)) => {}
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.subscribers.remove(&client_id);
            }
        }
    }

    fn reset_terminal_baseline(&mut self, client_id: u64, bootstrap: &SessionBootstrap) {
        let Some(subscriber) = self.subscribers.get_mut(&client_id) else {
            return;
        };
        subscriber.writer.clear_render();
        subscriber.pending_terminals.clear();
        subscriber.terminal_baselines = bootstrap
            .terminals
            .iter()
            .map(|terminal| (terminal.pane_id, terminal.view.clone()))
            .collect();
    }
}

struct LayoutEffect {
    started_terminal: Option<(PaneId, mpsc::Receiver<TerminalUpdate>, TerminalAgentProbe)>,
    removed_terminals: Vec<TerminalRuntime>,
}

enum ExternalLayoutPlan {
    CreateWorkspace {
        root: PathBuf,
    },
    CreateWorktree {
        parent_workspace_id: WorkspaceId,
        parent_root: PathBuf,
        branch: String,
        worktree_root: PathBuf,
    },
    OpenWorktree {
        parent_workspace_id: WorkspaceId,
        parent_root: PathBuf,
        root: PathBuf,
    },
    RemoveWorktree {
        workspace_id: WorkspaceId,
        parent_root: PathBuf,
        root: PathBuf,
        #[cfg(windows)]
        pane_ids: Vec<PaneId>,
    },
}

enum PreparedExternalLayout {
    CreateWorkspace {
        root: PathBuf,
        git: Option<GitRepository>,
    },
    CreateWorktree {
        parent_workspace_id: WorkspaceId,
        parent_root: PathBuf,
        parent: GitRepository,
        child: GitRepository,
        runtime: TerminalRuntime,
        updates: mpsc::Receiver<TerminalUpdate>,
    },
    OpenWorktree {
        parent_workspace_id: WorkspaceId,
        parent_root: PathBuf,
        child: GitRepository,
    },
    RemoveWorktreeReady {
        workspace_id: WorkspaceId,
        parent: GitRepository,
        child: GitRepository,
        #[cfg(windows)]
        pane_ids: Vec<PaneId>,
    },
    RemoveWorktree {
        workspace_id: WorkspaceId,
    },
}

fn external_layout_plan(
    state: &RuntimeState,
    command: &LayoutCommand,
) -> Result<Option<ExternalLayoutPlan>, String> {
    let plan = match command {
        LayoutCommand::CreateWorkspace { root_directory } => ExternalLayoutPlan::CreateWorkspace {
            root: root_directory.clone(),
        },
        LayoutCommand::CreateWorktree {
            parent_workspace_id,
            branch,
        } => {
            let parent_root = state
                .session
                .workspace(*parent_workspace_id)
                .ok_or_else(|| "unknown parent Workspace".to_string())?
                .root_directory()
                .to_path_buf();
            let worktree_root = state.worktree_root.clone().ok_or_else(|| {
                "cannot determine the managed worktree directory for this user".to_string()
            })?;
            ExternalLayoutPlan::CreateWorktree {
                parent_workspace_id: *parent_workspace_id,
                parent_root,
                branch: branch.clone(),
                worktree_root,
            }
        }
        LayoutCommand::OpenWorktree {
            parent_workspace_id,
            root_directory,
        } => {
            let parent_root = state
                .session
                .workspace(*parent_workspace_id)
                .ok_or_else(|| "unknown parent Workspace".to_string())?
                .root_directory()
                .to_path_buf();
            ExternalLayoutPlan::OpenWorktree {
                parent_workspace_id: *parent_workspace_id,
                parent_root,
                root: root_directory.clone(),
            }
        }
        LayoutCommand::RemoveWorktree { workspace_id } => {
            let workspace = state
                .session
                .workspace(*workspace_id)
                .ok_or_else(|| "unknown Workspace".to_string())?;
            let association = workspace
                .worktree()
                .filter(|association| association.is_managed())
                .ok_or_else(|| "Murmur can only remove worktrees it created".to_string())?;
            ExternalLayoutPlan::RemoveWorktree {
                workspace_id: *workspace_id,
                parent_root: association.parent_root_directory().to_path_buf(),
                root: workspace.root_directory().to_path_buf(),
                #[cfg(windows)]
                pane_ids: workspace
                    .tabs()
                    .iter()
                    .flat_map(|tab| tab.panes())
                    .map(|pane| pane.id())
                    .collect(),
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(plan))
}

fn prepare_external_layout(plan: ExternalLayoutPlan) -> Result<PreparedExternalLayout, String> {
    match plan {
        ExternalLayoutPlan::CreateWorkspace { root } => {
            Ok(PreparedExternalLayout::CreateWorkspace {
                git: discover_repository(&root).ok().flatten(),
                root,
            })
        }
        ExternalLayoutPlan::CreateWorktree {
            parent_workspace_id,
            parent_root,
            branch,
            worktree_root,
        } => {
            let parent = discover_repository(&parent_root)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "parent Workspace is not a Git repository".to_string())?;
            let child = create_worktree(&parent, &branch, &worktree_root)
                .map_err(|error| error.to_string())?;
            let mut runtime =
                match TerminalRuntime::spawn_shell(child.root(), TerminalSize::new(24, 80)) {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        let _ = remove_worktree(&parent, &child);
                        return Err(format!("failed to start terminal: {error}"));
                    }
                };
            let updates = runtime
                .take_updates()
                .expect("new Terminal update receiver exists");
            Ok(PreparedExternalLayout::CreateWorktree {
                parent_workspace_id,
                parent_root,
                parent,
                child,
                runtime,
                updates,
            })
        }
        ExternalLayoutPlan::OpenWorktree {
            parent_workspace_id,
            parent_root,
            root,
        } => {
            let parent = discover_repository(&parent_root)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "parent Workspace is not a Git repository".to_string())?;
            let child = open_worktree(&parent, root).map_err(|error| error.to_string())?;
            Ok(PreparedExternalLayout::OpenWorktree {
                parent_workspace_id,
                parent_root,
                child,
            })
        }
        ExternalLayoutPlan::RemoveWorktree {
            workspace_id,
            parent_root,
            root,
            #[cfg(windows)]
            pane_ids,
        } => {
            let parent = discover_repository(parent_root)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "parent Workspace is not a Git repository".to_string())?;
            let child = discover_repository(root)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| "Workspace is not a Git worktree".to_string())?;
            validate_worktree_removal(&parent, &child).map_err(|error| error.to_string())?;
            Ok(PreparedExternalLayout::RemoveWorktreeReady {
                workspace_id,
                parent,
                child,
                #[cfg(windows)]
                pane_ids,
            })
        }
    }
}

fn approve_external_layout(
    state: &mut RuntimeState,
    prepared: &PreparedExternalLayout,
) -> Result<(), String> {
    match prepared {
        PreparedExternalLayout::CreateWorktree {
            parent_workspace_id,
            parent_root,
            ..
        }
        | PreparedExternalLayout::OpenWorktree {
            parent_workspace_id,
            parent_root,
            ..
        } => {
            state
                .session
                .workspace(*parent_workspace_id)
                .filter(|workspace| workspace.root_directory() == parent_root)
                .ok_or_else(|| {
                    "parent Workspace changed while preparing its worktree".to_string()
                })?;
        }
        PreparedExternalLayout::RemoveWorktreeReady {
            workspace_id,
            child,
            #[cfg(windows)]
            pane_ids,
            ..
        } => {
            state
                .session
                .workspace(*workspace_id)
                .filter(|workspace| {
                    workspace.root_directory() == child.root()
                        && workspace
                            .worktree()
                            .is_some_and(|association| association.is_managed())
                })
                .ok_or_else(|| "managed Workspace changed while preparing removal".to_string())?;
            #[cfg(windows)]
            for pane_id in pane_ids {
                if let Some(runtime) = state.terminals.get_mut(pane_id) {
                    let _ = runtime.shutdown();
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn finish_external_layout(
    prepared: PreparedExternalLayout,
) -> Result<PreparedExternalLayout, String> {
    match prepared {
        PreparedExternalLayout::RemoveWorktreeReady {
            workspace_id,
            parent,
            child,
            ..
        } => {
            remove_worktree(&parent, &child).map_err(|error| error.to_string())?;
            Ok(PreparedExternalLayout::RemoveWorktree { workspace_id })
        }
        prepared => Ok(prepared),
    }
}

fn cancel_prepared_external_layout(prepared: PreparedExternalLayout) {
    if let PreparedExternalLayout::CreateWorktree {
        parent,
        child,
        runtime,
        updates,
        ..
    } = prepared
    {
        drop(updates);
        drop(runtime);
        let _ = remove_worktree(&parent, &child);
    }
}

fn apply_layout_command(
    state: &mut RuntimeState,
    command: LayoutCommand,
) -> Result<LayoutEffect, String> {
    let mut candidate = state.session.clone();
    let mut new_pane = None;
    let mut closed = None;

    match command {
        LayoutCommand::CreateWorkspace { root_directory } => {
            let workspace_id = candidate.create_workspace(root_directory);
            new_pane = Some(
                candidate
                    .workspace(workspace_id)
                    .expect("new Workspace exists")
                    .active_tab()
                    .focused_pane()
                    .id(),
            );
        }
        LayoutCommand::CreateWorktree {
            parent_workspace_id: _,
            branch: _,
        } => return Err("worktree command was not prepared".into()),
        LayoutCommand::OpenWorktree {
            parent_workspace_id: _,
            root_directory: _,
        }
        | LayoutCommand::RemoveWorktree { workspace_id: _ } => {
            return Err("worktree command was not prepared".into());
        }
        LayoutCommand::CreateTab { workspace_id } => {
            let tab_id = candidate
                .create_tab(workspace_id)
                .ok_or_else(|| "unknown Workspace".to_string())?;
            new_pane = Some(
                candidate
                    .tab(tab_id)
                    .expect("new Tab exists")
                    .focused_pane()
                    .id(),
            );
        }
        LayoutCommand::RenameWorkspace { workspace_id, name } => {
            if !candidate.rename_workspace(workspace_id, name) {
                return Err("unknown Workspace or empty name".into());
            }
        }
        LayoutCommand::RenameTab { tab_id, name } => {
            if !candidate.rename_tab(tab_id, name) {
                return Err("unknown Tab or empty name".into());
            }
        }
        LayoutCommand::ActivateWorkspace { workspace_id } => {
            if !candidate.activate_workspace(workspace_id) {
                return Err("unknown Workspace".into());
            }
        }
        LayoutCommand::ActivateTab { tab_id } => {
            if !candidate.activate_tab(tab_id) {
                return Err("unknown Tab".into());
            }
        }
        LayoutCommand::MoveWorkspace {
            workspace_id,
            target_index,
        } => {
            if candidate.workspace(workspace_id).is_none() {
                return Err("unknown Workspace".into());
            }
            if target_index as usize >= candidate.workspaces().len() {
                return Err("Workspace target index is out of range".into());
            }
            candidate.move_workspace(workspace_id, target_index as usize);
        }
        LayoutCommand::MoveTab {
            tab_id,
            target_index,
        } => {
            let Some(workspace) = candidate
                .workspaces()
                .iter()
                .find(|workspace| workspace.tabs().iter().any(|tab| tab.id() == tab_id))
            else {
                return Err("unknown Tab".into());
            };
            if target_index as usize >= workspace.tabs().len() {
                return Err("Tab target index is out of range".into());
            }
            candidate.move_tab(tab_id, target_index as usize);
        }
        LayoutCommand::SplitPane { pane_id, direction } => {
            new_pane = Some(
                candidate
                    .split_pane(pane_id, direction, 0.5)
                    .ok_or_else(|| "unknown Pane".to_string())?,
            );
        }
        LayoutCommand::FocusPane { pane_id } => {
            if !candidate.focus_pane(pane_id) {
                return Err("unknown Pane".into());
            }
        }
        LayoutCommand::FocusPaneDirection { pane_id, direction } => {
            if candidate.pane(pane_id).is_none() {
                return Err("unknown Pane".into());
            }
            candidate.focus_pane_in_direction(pane_id, direction);
        }
        LayoutCommand::ResizePane {
            pane_id,
            direction,
            amount,
        } => {
            if candidate.pane(pane_id).is_none() {
                return Err("unknown Pane".into());
            }
            if !amount.is_finite() || amount <= 0.0 {
                return Err("Pane resize amount must be a positive finite number".into());
            }
            candidate.resize_pane(pane_id, direction, amount);
        }
        LayoutCommand::SetSplitRatios { tab_id, ratios } => {
            let tab = candidate
                .tab(tab_id)
                .ok_or_else(|| "unknown Tab".to_string())?;
            if ratios.iter().any(|ratio| !ratio.is_finite()) {
                return Err("split ratios must be finite".into());
            }
            if ratios.len() != tab.panes().len().saturating_sub(1) {
                return Err("split ratio count does not match the Tab layout".into());
            }
            candidate.set_tab_split_ratios(tab_id, &ratios);
        }
        LayoutCommand::SwapPane { pane_id, direction } => {
            if candidate.pane(pane_id).is_none() {
                return Err("unknown Pane".into());
            }
            candidate.swap_pane(pane_id, direction);
        }
        LayoutCommand::TogglePaneZoom { pane_id } => {
            if candidate.pane(pane_id).is_none() {
                return Err("unknown Pane".into());
            }
            candidate.toggle_pane_zoom(pane_id);
        }
        LayoutCommand::ClosePane { pane_id } => {
            closed = Some(
                candidate
                    .close_pane(pane_id)
                    .ok_or_else(|| "unknown Pane".to_string())?,
            );
        }
        LayoutCommand::CloseTab { tab_id } => {
            closed = Some(
                candidate
                    .close_tab(tab_id)
                    .ok_or_else(|| "unknown Tab".to_string())?,
            );
        }
        LayoutCommand::CloseWorkspace { workspace_id } => {
            closed = Some(
                candidate
                    .close_workspace(workspace_id)
                    .ok_or_else(|| "unknown Workspace".to_string())?,
            );
        }
    }

    let started = if let Some(pane_id) = new_pane {
        let cwd = candidate
            .pane(pane_id)
            .and_then(|pane| pane.cwd())
            .ok_or_else(|| "new Pane has no working directory".to_string())?;
        let mut runtime = TerminalRuntime::spawn_shell(cwd, TerminalSize::new(24, 80))
            .map_err(|error| format!("failed to start terminal: {error}"))?;
        let updates = runtime
            .take_updates()
            .expect("new Terminal update receiver exists");
        Some((pane_id, runtime, updates))
    } else {
        None
    };

    Ok(commit_layout_candidate(state, candidate, closed, started))
}

fn commit_layout_candidate(
    state: &mut RuntimeState,
    candidate: Session,
    closed: Option<murmur_core::CloseOutcome>,
    started: Option<(PaneId, TerminalRuntime, mpsc::Receiver<TerminalUpdate>)>,
) -> LayoutEffect {
    state.session = candidate;
    let session = &state.session;
    state
        .workspace_git
        .retain(|workspace_id, _| session.workspace(*workspace_id).is_some());
    state
        .workspace_git_scanned_at
        .retain(|workspace_id, _| session.workspace(*workspace_id).is_some());
    let mut removed_terminals = Vec::new();
    if let Some(closed) = closed {
        for pane_id in closed.panes() {
            state.exited_terminals.remove(pane_id);
            state.agents.remove(pane_id);
            if let Some(runtime) = state.terminals.remove(pane_id) {
                removed_terminals.push(runtime);
            }
        }
    }
    let started_terminal = started.map(|(pane_id, runtime, updates)| {
        let probe = runtime
            .agent_probe()
            .expect("new Terminal has an agent probe");
        state.terminals.insert(pane_id, runtime);
        (pane_id, updates, probe)
    });
    LayoutEffect {
        started_terminal,
        removed_terminals,
    }
}

fn apply_prepared_external_layout(
    state: &mut RuntimeState,
    prepared: PreparedExternalLayout,
) -> Result<LayoutEffect, String> {
    match prepared {
        PreparedExternalLayout::CreateWorkspace { root, git } => {
            let effect = apply_layout_command(
                state,
                LayoutCommand::CreateWorkspace {
                    root_directory: root,
                },
            )?;
            let workspace_id = state
                .session
                .active_workspace_id()
                .expect("created Workspace is active");
            set_workspace_git(state, workspace_id, git);
            Ok(effect)
        }
        PreparedExternalLayout::CreateWorktree {
            parent_workspace_id,
            parent_root,
            parent: _,
            child,
            runtime,
            updates,
        } => {
            let mut candidate = state.session.clone();
            if candidate
                .workspace(parent_workspace_id)
                .is_none_or(|workspace| workspace.root_directory() != parent_root)
            {
                return Err("parent Workspace changed while creating its worktree".into());
            }
            let workspace_id = candidate.create_workspace(child.root().to_path_buf());
            if !candidate.associate_worktree(workspace_id, parent_workspace_id, parent_root, true) {
                return Err("failed to associate the created worktree".into());
            }
            let pane_id = candidate
                .workspace(workspace_id)
                .expect("created Workspace exists")
                .active_tab()
                .focused_pane()
                .id();
            let effect =
                commit_layout_candidate(state, candidate, None, Some((pane_id, runtime, updates)));
            set_workspace_git(state, workspace_id, Some(child));
            Ok(effect)
        }
        PreparedExternalLayout::OpenWorktree {
            parent_workspace_id,
            parent_root,
            child,
        } => {
            let mut candidate = state.session.clone();
            if candidate
                .workspace(parent_workspace_id)
                .is_none_or(|workspace| workspace.root_directory() != parent_root)
            {
                return Err("parent Workspace changed while opening its worktree".into());
            }
            let (workspace_id, started) = if let Some(workspace_id) = candidate
                .workspace_by_root(child.root())
                .map(|workspace| workspace.id())
            {
                if candidate
                    .workspace(workspace_id)
                    .is_some_and(|workspace| workspace.worktree().is_none())
                {
                    candidate.associate_worktree(
                        workspace_id,
                        parent_workspace_id,
                        parent_root,
                        false,
                    );
                }
                candidate.activate_workspace(workspace_id);
                (workspace_id, None)
            } else {
                let workspace_id = candidate.create_workspace(child.root().to_path_buf());
                candidate.associate_worktree(workspace_id, parent_workspace_id, parent_root, false);
                let pane_id = candidate
                    .workspace(workspace_id)
                    .expect("created Workspace exists")
                    .active_tab()
                    .focused_pane()
                    .id();
                let mut runtime =
                    TerminalRuntime::spawn_shell(child.root(), TerminalSize::new(24, 80))
                        .map_err(|error| format!("failed to start terminal: {error}"))?;
                let updates = runtime
                    .take_updates()
                    .expect("new Terminal update receiver exists");
                (workspace_id, Some((pane_id, runtime, updates)))
            };
            let effect = commit_layout_candidate(state, candidate, None, started);
            set_workspace_git(state, workspace_id, Some(child));
            Ok(effect)
        }
        PreparedExternalLayout::RemoveWorktreeReady { .. } => {
            Err("worktree removal was not finalized".into())
        }
        PreparedExternalLayout::RemoveWorktree { workspace_id } => {
            let mut candidate = state.session.clone();
            let workspace = candidate
                .workspace(workspace_id)
                .filter(|workspace| {
                    workspace
                        .worktree()
                        .is_some_and(|association| association.is_managed())
                })
                .ok_or_else(|| {
                    "managed Workspace changed while removing its worktree".to_string()
                })?;
            let _ = workspace;
            let closed = candidate
                .close_workspace(workspace_id)
                .expect("validated Workspace exists");
            Ok(commit_layout_candidate(
                state,
                candidate,
                Some(closed),
                None,
            ))
        }
    }
}

fn set_workspace_git(
    state: &mut RuntimeState,
    workspace_id: WorkspaceId,
    git: Option<GitRepository>,
) {
    state
        .workspace_git_scanned_at
        .insert(workspace_id, Instant::now());
    match git {
        Some(repository) => {
            state.workspace_git.insert(workspace_id, repository);
        }
        None => {
            state.workspace_git.remove(&workspace_id);
        }
    }
}

fn layout_authority_error(
    state: &RuntimeState,
    client_id: u64,
    server_id: ServerId,
    session_id: SessionId,
    stopping: bool,
) -> Option<ServerMessage> {
    if stopping {
        Some(ServerMessage::Error {
            message: "Server is stopping".into(),
        })
    } else if server_id != state.server_id {
        Some(ServerMessage::Error {
            message: "unknown Server".into(),
        })
    } else if session_id != state.session_id {
        Some(ServerMessage::Error {
            message: "unknown Session".into(),
        })
    } else if state.active_controller != Some(client_id) {
        Some(ServerMessage::ControlDenied {
            server_id: state.server_id,
            session_id,
            reason: "acquire Session control before mutating layout".into(),
        })
    } else {
        None
    }
}

fn plan_client_external_layout(
    state: &mut RuntimeState,
    client_id: u64,
    server_id: ServerId,
    session_id: SessionId,
    stopping: bool,
    command: &LayoutCommand,
) -> Result<ExternalLayoutPlan, Box<ServerMessage>> {
    if let Some(error) = layout_authority_error(state, client_id, server_id, session_id, stopping) {
        return Err(Box::new(error));
    }
    let plan = external_layout_plan(state, command)
        .map_err(|message| Box::new(ServerMessage::Error { message }))?
        .expect("external Layout command has a plan");
    Ok(plan)
}

fn handle_client(
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
    let hello = match murmur_core::protocol::read_message::<_, ClientMessage>(&mut stream) {
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
    if send_bootstrap(&mut stream, &state).is_err() {
        return;
    }
    let _ = stream.set_handshake_timeout(None);

    let mut writer_stream = match stream.try_clone() {
        Ok(stream) => stream,
        Err(_) => return,
    };
    let (outbound, outbound_rx) = ClientWriter::channel();
    let writer_state = Arc::downgrade(&state);
    let writer = thread::spawn(move || {
        while let Some(item) = outbound_rx.recv() {
            let (data, render) = match item {
                ClientWriteItem::Reliable(data) => (data, false),
                ClientWriteItem::Render(data) => (data, true),
            };
            if send_framed(&mut writer_stream, &data).is_err() {
                break;
            }
            if render && let Some(state) = writer_state.upgrade() {
                state
                    .lock()
                    .expect("server state lock poisoned")
                    .flush_terminal_render(client_id, None);
            }
        }
        outbound_rx.close();
    });

    let mut stopping_server = false;
    loop {
        let message = match murmur_core::protocol::read_message(&mut stream) {
            Ok(message) => message,
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
        let mut started_terminal = None;
        let mut removed_terminals = Vec::new();
        let should_close = match message {
            ClientMessage::SnapshotRequest { session_id } => {
                let mut state = state.lock().expect("server state lock poisoned");
                if session_id != state.session_id {
                    queue_message(
                        &outbound,
                        ServerMessage::Error {
                            message: "unknown Session".into(),
                        },
                    )
                } else {
                    let bootstrap = state.bootstrap();
                    state.reset_terminal_baseline(client_id, &bootstrap);
                    queue_message(&outbound, ServerMessage::Bootstrap(bootstrap))
                }
            }
            ClientMessage::Subscribe {
                session_id,
                after_sequence,
            } => {
                let mut state = state.lock().expect("server state lock poisoned");
                let (responses, should_subscribe) = {
                    if session_id != state.session_id {
                        (
                            vec![ServerMessage::Error {
                                message: "unknown Session".into(),
                            }],
                            false,
                        )
                    } else if after_sequence > state.sequence {
                        (
                            vec![ServerMessage::Error {
                                message: "event cursor is ahead of the server".into(),
                            }],
                            false,
                        )
                    } else if state
                        .events
                        .front()
                        .is_some_and(|event| after_sequence.saturating_add(1) < event.sequence)
                    {
                        (
                            vec![ServerMessage::Error {
                                message: "event cursor expired; request a fresh Bootstrap".into(),
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
                    state.subscribers.insert(
                        client_id,
                        ClientSubscriber {
                            writer: outbound.clone(),
                            terminal_baselines: std::collections::HashMap::new(),
                            pending_terminals: std::collections::HashSet::new(),
                        },
                    );
                }
                let failed = responses
                    .into_iter()
                    .any(|response| queue_message(&outbound, response));
                if failed {
                    state.subscribers.remove(&client_id);
                } else if should_subscribe {
                    let pane_ids = state.terminals.keys().copied().collect::<Vec<_>>();
                    state
                        .subscribers
                        .get_mut(&client_id)
                        .expect("new subscriber exists")
                        .pending_terminals
                        .extend(pane_ids);
                    state.flush_terminal_render(client_id, None);
                }
                failed
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
            ClientMessage::AcquireControl { session_id } => {
                let response = {
                    let mut state = state.lock().expect("server state lock poisoned");
                    if session_id != state.session_id {
                        ServerMessage::ControlDenied {
                            server_id: state.server_id,
                            session_id,
                            reason: "unknown Session".into(),
                        }
                    } else if state.active_controller.is_none()
                        || state.active_controller == Some(client_id)
                    {
                        state.active_controller = Some(client_id);
                        ServerMessage::ControlGranted {
                            server_id: state.server_id,
                            session_id,
                        }
                    } else {
                        ServerMessage::ControlDenied {
                            server_id: state.server_id,
                            session_id,
                            reason: "another client controls this Session".into(),
                        }
                    }
                };
                queue_message(&outbound, response)
            }
            ClientMessage::ReleaseControl { session_id } => {
                let response = {
                    let mut state = state.lock().expect("server state lock poisoned");
                    if state.session_id != session_id {
                        ServerMessage::Error {
                            message: "unknown Session".into(),
                        }
                    } else if state.active_controller == Some(client_id) {
                        state.active_controller = None;
                        ServerMessage::ControlReleased {
                            server_id: state.server_id,
                            session_id,
                        }
                    } else {
                        ServerMessage::ControlDenied {
                            server_id: state.server_id,
                            session_id,
                            reason: "client does not control this Session".into(),
                        }
                    }
                };
                queue_message(&outbound, response)
            }
            ClientMessage::Layout {
                server_id,
                session_id,
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
                    let operation = lifecycle.begin_external();
                    let plan = if operation.is_some() {
                        let mut state = state.lock().expect("server state lock poisoned");
                        plan_client_external_layout(
                            &mut state,
                            client_id,
                            server_id,
                            session_id,
                            lifecycle.is_stopping(),
                            &command,
                        )
                    } else {
                        Err(Box::new(ServerMessage::Error {
                            message: "Server is stopping".into(),
                        }))
                    };
                    match plan.and_then(|plan| {
                        prepare_external_layout(plan)
                            .map_err(|message| Box::new(ServerMessage::Error { message }))
                    }) {
                        Err(message) => queue_message(&outbound, *message),
                        Ok(prepared) => {
                            let approval = {
                                let mut state = state.lock().expect("server state lock poisoned");
                                if let Some(message) = layout_authority_error(
                                    &state,
                                    client_id,
                                    server_id,
                                    session_id,
                                    lifecycle.is_stopping(),
                                ) {
                                    Err(Box::new(message))
                                } else {
                                    approve_external_layout(&mut state, &prepared).map_err(
                                        |message| Box::new(ServerMessage::Error { message }),
                                    )
                                }
                            };
                            match approval {
                                Err(message) => {
                                    cancel_prepared_external_layout(prepared);
                                    queue_message(&outbound, *message)
                                }
                                Ok(()) => match finish_external_layout(prepared) {
                                    Err(message) => {
                                        queue_message(&outbound, ServerMessage::Error { message })
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
                                            &state, client_id, server_id, session_id, stopping,
                                        ) {
                                            drop(state);
                                            cancel_prepared_external_layout(prepared);
                                            queue_message(&outbound, message)
                                        } else {
                                            match apply_prepared_external_layout(
                                                &mut state, prepared,
                                            ) {
                                                Ok(effect) => {
                                                    started_terminal = effect.started_terminal;
                                                    removed_terminals = effect.removed_terminals;
                                                    state
                                                        .publish_layout_change(client_id, &outbound)
                                                }
                                                Err(message) => queue_message(
                                                    &outbound,
                                                    ServerMessage::Error { message },
                                                ),
                                            }
                                        }
                                    }
                                },
                            }
                        }
                    }
                } else {
                    let mut state = state.lock().expect("server state lock poisoned");
                    if let Some(message) = layout_authority_error(
                        &state,
                        client_id,
                        server_id,
                        session_id,
                        lifecycle.is_stopping(),
                    ) {
                        queue_message(&outbound, message)
                    } else {
                        match apply_layout_command(&mut state, command) {
                            Ok(effect) => {
                                started_terminal = effect.started_terminal;
                                removed_terminals = effect.removed_terminals;
                                state.publish_layout_change(client_id, &outbound)
                            }
                            Err(message) => {
                                queue_message(&outbound, ServerMessage::Error { message })
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
                let state = state.lock().expect("server state lock poisoned");
                let is_copy = matches!(&command, TerminalCommand::Copy { .. });
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
                } else if !is_copy && state.active_controller != Some(client_id) {
                    queue_message(
                        &outbound,
                        ServerMessage::ControlDenied {
                            server_id,
                            session_id,
                            reason: "acquire Session control before using a terminal".into(),
                        },
                    )
                } else if state.exited_terminals.contains(&pane_id) {
                    queue_message(
                        &outbound,
                        ServerMessage::Error {
                            message: "terminal has exited".into(),
                        },
                    )
                } else {
                    let result = state
                        .terminals
                        .get(&pane_id)
                        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unknown Pane"))
                        .and_then(|runtime| runtime.execute(command));
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
                    let _ = queue_message(&outbound, ServerMessage::ServerStopping);
                    stopping_server = true;
                    true
                }
            }
            ClientMessage::Detach => true,
            ClientMessage::Hello(Hello { .. }) => false,
        };
        if let Some((pane_id, updates, probe)) = started_terminal {
            monitor_terminal(pane_id, updates, probe, Arc::clone(&state));
        }
        drop(removed_terminals);
        if should_close {
            break;
        }
    }

    let mut state = state.lock().expect("server state lock poisoned");
    state.subscribers.remove(&client_id);
    if state.active_controller == Some(client_id) {
        state.active_controller = None;
    }
    drop(state);
    drop(outbound);
    let _ = writer.join();
    if stopping_server {
        stop.store(true, Ordering::Release);
    }
}

fn monitor_terminal(
    pane_id: PaneId,
    updates: mpsc::Receiver<TerminalUpdate>,
    probe: TerminalAgentProbe,
    state: Arc<Mutex<RuntimeState>>,
) {
    thread::spawn(move || {
        let mut last_agent_scan = Instant::now();
        let mut last_view_publish = Instant::now()
            .checked_sub(TERMINAL_FRAME_INTERVAL)
            .unwrap_or_else(Instant::now);
        let mut agent_scan_pending = false;
        let mut git_scan_pending = None;
        loop {
            let update = if agent_scan_pending || git_scan_pending.is_some() {
                match updates.recv_timeout(AGENT_SCAN_INTERVAL) {
                    Ok(update) => Some(update),
                    Err(mpsc::RecvTimeoutError::Timeout) => None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            } else {
                match updates.recv() {
                    Ok(update) => Some(update),
                    Err(_) => break,
                }
            };
            let update = update.map(|update| {
                coalesce_terminal_update(
                    update,
                    &updates,
                    last_view_publish + TERMINAL_FRAME_INTERVAL,
                )
            });
            match update {
                Some(TerminalUpdate::View(_)) => {
                    agent_scan_pending = true;
                    git_scan_pending = Some(Instant::now());
                    let mut state = state.lock().expect("server state lock poisoned");
                    let Some(view) = state.terminals.get(&pane_id).map(TerminalRuntime::view)
                    else {
                        break;
                    };
                    state.publish_terminal(pane_id, &view);
                    last_view_publish = Instant::now();
                }
                Some(TerminalUpdate::Exited) => {
                    let mut state = state.lock().expect("server state lock poisoned");
                    let Some(view) = state.terminals.get_mut(&pane_id).map(|runtime| {
                        let _ = runtime.wait();
                        runtime.view()
                    }) else {
                        break;
                    };
                    state.exited_terminals.insert(pane_id);
                    state.publish_terminal(pane_id, &view);
                    state.publish_background(SessionEvent::TerminalExited { pane_id });
                    if state.agents.remove(&pane_id).is_some() {
                        state.publish_background(SessionEvent::AgentChanged {
                            pane_id,
                            agent: None,
                        });
                    }
                    break;
                }
                None => {}
            }

            let now = Instant::now();
            if agent_scan_pending && now.duration_since(last_agent_scan) >= AGENT_SCAN_INTERVAL {
                let previous = {
                    let state = state.lock().expect("server state lock poisoned");
                    if !state.terminals.contains_key(&pane_id) {
                        break;
                    }
                    state.agents.get(&pane_id).copied()
                };
                let next = probe.snapshot(previous);
                let mut state = state.lock().expect("server state lock poisoned");
                if !state.terminals.contains_key(&pane_id) {
                    break;
                }
                apply_agent_refresh(&mut state, pane_id, next);
                last_agent_scan = now;
                agent_scan_pending = false;
            }
            if let Some(activity) = git_scan_pending {
                let scan = {
                    let mut state = state.lock().expect("server state lock poisoned");
                    reserve_workspace_git_scan(&mut state, pane_id, activity, now)
                };
                match scan {
                    WorkspaceGitScan::Waiting => {}
                    WorkspaceGitScan::Covered => git_scan_pending = None,
                    WorkspaceGitScan::Ready { workspace_id, root } => {
                        git_scan_pending = None;
                        if let Ok(next) = discover_repository(&root) {
                            let mut state = state.lock().expect("server state lock poisoned");
                            apply_workspace_git_refresh(&mut state, workspace_id, &root, next);
                        }
                    }
                }
            }
        }
    });
}

fn coalesce_terminal_update(
    first: TerminalUpdate,
    updates: &mpsc::Receiver<TerminalUpdate>,
    publish_at: Instant,
) -> TerminalUpdate {
    let TerminalUpdate::View(mut revision) = first else {
        return first;
    };

    loop {
        let next = if let Some(remaining) = publish_at.checked_duration_since(Instant::now()) {
            match updates.recv_timeout(remaining) {
                Ok(update) => Some(update),
                Err(mpsc::RecvTimeoutError::Timeout) => None,
                Err(mpsc::RecvTimeoutError::Disconnected) => return TerminalUpdate::View(revision),
            }
        } else {
            updates.try_recv().ok()
        };

        match next {
            Some(TerminalUpdate::View(next_revision)) => revision = next_revision,
            Some(TerminalUpdate::Exited) => return TerminalUpdate::Exited,
            None => return TerminalUpdate::View(revision),
        }
    }
}

fn apply_agent_refresh(state: &mut RuntimeState, pane_id: PaneId, next: Option<AgentSnapshot>) {
    let previous = state.agents.get(&pane_id).copied();
    if previous == next {
        return;
    }
    match next {
        Some(agent) => {
            state.agents.insert(pane_id, agent);
        }
        None => {
            state.agents.remove(&pane_id);
        }
    }
    state.publish_background(SessionEvent::AgentChanged {
        pane_id,
        agent: next,
    });
}

enum WorkspaceGitScan {
    Waiting,
    Covered,
    Ready {
        workspace_id: WorkspaceId,
        root: PathBuf,
    },
}

fn reserve_workspace_git_scan(
    state: &mut RuntimeState,
    pane_id: PaneId,
    activity: Instant,
    now: Instant,
) -> WorkspaceGitScan {
    let Some(workspace) = state.session.workspace_for_pane(pane_id) else {
        return WorkspaceGitScan::Covered;
    };
    let workspace_id = workspace.id();
    let root = workspace.root_directory().to_path_buf();
    if let Some(scanned_at) = state.workspace_git_scanned_at.get(&workspace_id) {
        if *scanned_at >= activity {
            return WorkspaceGitScan::Covered;
        }
        if now.duration_since(*scanned_at) < GIT_SCAN_INTERVAL {
            return WorkspaceGitScan::Waiting;
        }
    }
    state.workspace_git_scanned_at.insert(workspace_id, now);
    WorkspaceGitScan::Ready { workspace_id, root }
}

fn apply_workspace_git_refresh(
    state: &mut RuntimeState,
    workspace_id: WorkspaceId,
    root: &std::path::Path,
    next: Option<GitRepository>,
) {
    if state
        .session
        .workspace(workspace_id)
        .is_none_or(|workspace| workspace.root_directory() != root)
    {
        return;
    }
    if state.workspace_git.get(&workspace_id) == next.as_ref() {
        return;
    }
    let git = match next {
        Some(repository) => {
            let snapshot = workspace_git_snapshot(workspace_id, &repository);
            state.workspace_git.insert(workspace_id, repository);
            Some(snapshot)
        }
        None => {
            state.workspace_git.remove(&workspace_id);
            None
        }
    };
    state.publish_background(SessionEvent::WorkspaceGitChanged { workspace_id, git });
}

fn workspace_git_snapshot(
    workspace_id: WorkspaceId,
    repository: &GitRepository,
) -> WorkspaceGitSnapshot {
    WorkspaceGitSnapshot {
        workspace_id,
        branch: repository.branch().map(str::to_owned),
        linked_worktree: repository.is_linked_worktree(),
    }
}

fn queue_message(outbound: &ClientWriter, message: ServerMessage) -> bool {
    frame_message(&message)
        .and_then(|data| {
            outbound
                .send_reliable(data)
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "client writer stopped"))
        })
        .is_err()
}

fn send_bootstrap(stream: &mut EndpointStream, state: &Arc<Mutex<RuntimeState>>) -> io::Result<()> {
    let bootstrap = state
        .lock()
        .expect("server state lock poisoned")
        .bootstrap();
    send_message(stream, &ServerMessage::Bootstrap(bootstrap))
}

fn send_error(
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

fn send_message(stream: &mut EndpointStream, message: &ServerMessage) -> io::Result<()> {
    murmur_core::protocol::write_message(stream, message)
        .map_err(|error| io::Error::other(error.to_string()))
}

fn frame_message(message: &ServerMessage) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    murmur_core::protocol::write_message(&mut data, message)
        .map_err(|error| io::Error::other(error.to_string()))?;
    Ok(data)
}

fn frame_terminal_batches(
    server_id: ServerId,
    session_id: SessionId,
    panes: Vec<PaneTerminalFrame>,
) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    let mut batch = Vec::new();
    for pane in panes {
        batch.push(pane);
        let message = ServerMessage::TerminalFrame(TerminalFrameBatch {
            server_id,
            session_id,
            panes: batch.clone(),
        });
        let error = match frame_message(&message) {
            Ok(_) => continue,
            Err(error) => error,
        };

        let last = batch.pop().expect("terminal render batch is non-empty");
        if batch.is_empty() {
            return Err(error);
        }
        data.extend(frame_message(&ServerMessage::TerminalFrame(
            TerminalFrameBatch {
                server_id,
                session_id,
                panes: std::mem::take(&mut batch),
            },
        ))?);
        batch.push(last);
    }
    if !batch.is_empty() {
        data.extend(frame_message(&ServerMessage::TerminalFrame(
            TerminalFrameBatch {
                server_id,
                session_id,
                panes: batch,
            },
        ))?);
    }
    Ok(data)
}

fn send_framed(stream: &mut EndpointStream, data: &[u8]) -> io::Result<()> {
    stream.write_all(data)?;
    stream.flush()
}

fn stable_endpoint_id(endpoint: &Endpoint) -> u64 {
    let text = match endpoint {
        Endpoint::Local(path) => format!("local:{}", path.to_string_lossy()),
        Endpoint::Tcp(address) => format!("tcp:{address}"),
    };
    text.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

fn runtime_epoch() -> u128 {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    (u128::from(std::process::id()) << 96) ^ nanos
}

pub fn ensure_local_server() -> io::Result<Endpoint> {
    let endpoint = Endpoint::local(default_socket_path());
    if let Ok(stream) = endpoint.connect() {
        match probe_protocol(stream) {
            Ok(()) => return Ok(endpoint),
            Err(error) if error.kind() == io::ErrorKind::InvalidData => return Err(error),
            Err(_) => {}
        }
    }

    let server_executable = resolve_server_executable()?;
    let mut command = std::process::Command::new(&server_executable);
    command
        .arg("--endpoint")
        .arg(endpoint.as_local_path().expect("local endpoint"))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command.spawn().map_err(|error| {
        io::Error::new(
            error.kind(),
            format!(
                "failed to start murmur-server at {}: {error}",
                server_executable.display()
            ),
        )
    })?;

    for _ in 0..150 {
        if let Ok(stream) = endpoint.connect()
            && probe_protocol(stream).is_ok()
        {
            return Ok(endpoint);
        }
        thread::sleep(Duration::from_millis(10));
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "murmur-server did not become ready",
    ))
}

fn resolve_server_executable() -> io::Result<PathBuf> {
    if let Some(path) = std::env::var_os("MURMUR_SERVER_EXECUTABLE") {
        let path = PathBuf::from(path);
        return path.is_file().then_some(path).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "MURMUR_SERVER_EXECUTABLE does not name a file",
            )
        });
    }

    let current_executable = std::env::current_exe()?;
    let server_name = if cfg!(windows) {
        "murmur-server.exe"
    } else {
        "murmur-server"
    };
    let sibling = current_executable
        .parent()
        .map(|parent| parent.join(server_name))
        .unwrap_or_else(|| PathBuf::from(server_name));
    sibling.is_file().then_some(sibling).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            "murmur-server is not installed beside the GUI; build or install the standalone server",
        )
    })
}

fn probe_protocol(stream: EndpointStream) -> io::Result<()> {
    let _ = ClientConnection::handshake(stream, "murmur-probe")?;
    Ok(())
}

pub fn stop_server(endpoint: &Endpoint) -> io::Result<()> {
    let client = ClientConnection::connect(endpoint, "murmur-stop")?;
    let server_id = client.bootstrap.server_id;
    let mut stream = client.into_stream();
    murmur_core::protocol::write_message(&mut stream, &ClientMessage::StopServer { server_id })
        .map_err(|error| io::Error::other(error.to_string()))?;
    match murmur_core::protocol::read_message(&mut stream)
        .map_err(|error| io::Error::other(error.to_string()))?
    {
        ServerMessage::ServerStopping => Ok(()),
        other => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unexpected stop response: {other:?}"),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_frame_coalescing_keeps_the_latest_revision() {
        let (sender, updates) = mpsc::channel();
        sender.send(TerminalUpdate::View(2)).unwrap();
        sender.send(TerminalUpdate::View(3)).unwrap();

        assert_eq!(
            coalesce_terminal_update(TerminalUpdate::View(1), &updates, Instant::now()),
            TerminalUpdate::View(3)
        );
    }

    #[test]
    fn terminal_exit_supersedes_queued_view_updates() {
        let (sender, updates) = mpsc::channel();
        sender.send(TerminalUpdate::View(2)).unwrap();
        sender.send(TerminalUpdate::Exited).unwrap();

        assert_eq!(
            coalesce_terminal_update(TerminalUpdate::View(1), &updates, Instant::now()),
            TerminalUpdate::Exited
        );
    }

    #[test]
    fn client_terminal_baseline_advances_only_for_an_accepted_render() {
        let mut session = Session::new();
        session.create_workspace(std::env::temp_dir());
        let pane_id = session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id();
        let view = |revision| TerminalView {
            revision,
            size: TerminalSize::new(1, 1),
            display_offset: 0,
            cells: Vec::new(),
            cursor: None,
        };
        let (writer, receiver) = ClientWriter::channel();
        let mut subscriber = ClientSubscriber {
            writer,
            terminal_baselines: std::collections::HashMap::new(),
            pending_terminals: std::collections::HashSet::new(),
        };

        subscriber
            .try_send_terminal_render(vec![1], vec![(pane_id, view(1))])
            .unwrap();
        assert_eq!(subscriber.terminal_baselines[&pane_id].revision, 1);
        assert!(matches!(
            subscriber.try_send_terminal_render(vec![2], vec![(pane_id, view(2))]),
            Err(mpsc::TrySendError::Full(_))
        ));
        assert_eq!(subscriber.terminal_baselines[&pane_id].revision, 1);

        assert_eq!(receiver.recv(), Some(ClientWriteItem::Render(vec![1])));
        subscriber
            .try_send_terminal_render(vec![3], vec![(pane_id, view(3))])
            .unwrap();
        assert_eq!(subscriber.terminal_baselines[&pane_id].revision, 3);
    }

    fn test_endpoint() -> Endpoint {
        Endpoint::local(std::env::temp_dir().join(format!(
            "murmur-server-{}-{}.sock",
            std::process::id(),
            unique_suffix()
        )))
    }

    fn unique_suffix() -> u128 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    }

    fn run_git(cwd: &std::path::Path, args: &[&str]) {
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

    fn start() -> (ServerHandle, Endpoint, thread::JoinHandle<io::Result<()>>) {
        let endpoint = test_endpoint();
        let server = BoundServer::bind(ServerConfig {
            endpoint: endpoint.clone(),
        })
        .unwrap();
        let handle = server.handle();
        let thread = thread::spawn(move || server.run());
        for _ in 0..100 {
            if endpoint.connect().is_ok() {
                return (handle, endpoint, thread);
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("server did not start");
    }

    fn connect_and_bootstrap(endpoint: &Endpoint) -> EndpointStream {
        let mut stream = endpoint.connect().unwrap();
        murmur_core::protocol::write_message(
            &mut stream,
            &ClientMessage::Hello(Hello {
                version: PROTOCOL_VERSION,
                client_name: "test".into(),
            }),
        )
        .unwrap();
        let welcome: ServerMessage = murmur_core::protocol::read_message(&mut stream).unwrap();
        assert!(matches!(
            welcome,
            ServerMessage::Welcome { error: None, .. }
        ));
        let bootstrap: ServerMessage = murmur_core::protocol::read_message(&mut stream).unwrap();
        assert!(matches!(bootstrap, ServerMessage::Bootstrap(_)));
        stream
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn apply_for_test(
        state: &mut RuntimeState,
        updates: &mut Vec<mpsc::Receiver<TerminalUpdate>>,
        command: LayoutCommand,
    ) -> usize {
        let plan = external_layout_plan(state, &command).unwrap();
        let effect = match plan {
            Some(plan) => {
                let prepared = prepare_external_layout(plan).unwrap();
                approve_external_layout(state, &prepared).unwrap();
                apply_prepared_external_layout(state, finish_external_layout(prepared).unwrap())
                    .unwrap()
            }
            None => apply_layout_command(state, command).unwrap(),
        };
        if let Some((_, receiver, _)) = effect.started_terminal {
            updates.push(receiver);
        }
        effect.removed_terminals.len()
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn layout_commands_keep_structure_zoom_and_terminals_in_sync() {
        use murmur_core::{PaneDirection, PaneLayout, SplitDirection};

        let mut state = RuntimeState::new(&test_endpoint());
        let mut updates = Vec::new();
        let root = std::env::temp_dir();

        assert_eq!(
            apply_for_test(
                &mut state,
                &mut updates,
                LayoutCommand::CreateWorkspace {
                    root_directory: root.clone(),
                },
            ),
            0
        );
        let workspace_id = state.session.active_workspace_id().unwrap();
        let tab_one = state.session.active_workspace().unwrap().active_tab().id();
        let pane_one = state
            .session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id();

        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::CreateTab { workspace_id },
        );
        let tab_two = state.session.active_workspace().unwrap().active_tab().id();
        let pane_two = state
            .session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id();
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::RenameWorkspace {
                workspace_id,
                name: "renamed workspace".into(),
            },
        );
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::RenameTab {
                tab_id: tab_two,
                name: "renamed tab".into(),
            },
        );
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::SplitPane {
                pane_id: pane_two,
                direction: SplitDirection::Horizontal,
            },
        );
        let pane_three = state.session.tab(tab_two).unwrap().focused_pane().id();
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::FocusPane { pane_id: pane_two },
        );
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::FocusPaneDirection {
                pane_id: pane_two,
                direction: PaneDirection::Right,
            },
        );
        assert_eq!(
            state.session.tab(tab_two).unwrap().focused_pane().id(),
            pane_three
        );
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::ResizePane {
                pane_id: pane_three,
                direction: PaneDirection::Left,
                amount: 0.05,
            },
        );

        let before_invalid_ratio = state.session.snapshot();
        assert!(
            apply_layout_command(
                &mut state,
                LayoutCommand::SetSplitRatios {
                    tab_id: tab_two,
                    ratios: Vec::new(),
                },
            )
            .is_err()
        );
        assert_eq!(state.session.snapshot(), before_invalid_ratio);
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::SetSplitRatios {
                tab_id: tab_two,
                ratios: vec![0.6],
            },
        );
        assert!(matches!(
            state.session.tab(tab_two).unwrap().layout(),
            PaneLayout::Split { ratio, .. } if (*ratio - 0.6).abs() < f32::EPSILON
        ));
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::SwapPane {
                pane_id: pane_three,
                direction: PaneDirection::Left,
            },
        );
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::TogglePaneZoom {
                pane_id: pane_three,
            },
        );
        assert_eq!(state.bootstrap().zoomed_panes, vec![pane_three]);

        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::CreateWorkspace {
                root_directory: root,
            },
        );
        let workspace_two = state.session.active_workspace_id().unwrap();
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::MoveWorkspace {
                workspace_id: workspace_two,
                target_index: 0,
            },
        );
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::ActivateWorkspace { workspace_id },
        );
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::MoveTab {
                tab_id: tab_two,
                target_index: 0,
            },
        );
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::ActivateTab { tab_id: tab_two },
        );

        assert_eq!(state.terminals.len(), 4);
        assert_eq!(
            apply_for_test(
                &mut state,
                &mut updates,
                LayoutCommand::ClosePane {
                    pane_id: pane_three,
                },
            ),
            1
        );
        assert_eq!(
            apply_for_test(
                &mut state,
                &mut updates,
                LayoutCommand::CloseTab { tab_id: tab_two },
            ),
            1
        );
        assert_eq!(
            apply_for_test(
                &mut state,
                &mut updates,
                LayoutCommand::CloseWorkspace { workspace_id },
            ),
            1
        );
        assert!(state.session.pane(pane_one).is_none());
        assert_eq!(state.terminals.len(), 1);
        assert_eq!(
            apply_for_test(
                &mut state,
                &mut updates,
                LayoutCommand::CloseWorkspace {
                    workspace_id: workspace_two,
                },
            ),
            1
        );
        assert!(state.session.is_empty());
        assert!(state.terminals.is_empty());
        assert!(state.bootstrap().zoomed_panes.is_empty());
        assert!(state.session.tab(tab_one).is_none());
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn layout_commands_create_and_remove_a_managed_worktree_without_deleting_its_branch() {
        let temp = std::env::temp_dir().join(format!(
            "murmur-server-worktree-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let repository = temp.join("repository");
        std::fs::create_dir_all(&repository).unwrap();
        run_git(&repository, &["init"]);
        run_git(&repository, &["config", "user.name", "Murmur Tests"]);
        run_git(
            &repository,
            &["config", "user.email", "murmur@example.invalid"],
        );
        std::fs::write(repository.join("README.md"), "murmur\n").unwrap();
        run_git(&repository, &["add", "README.md"]);
        run_git(&repository, &["commit", "-m", "initial"]);

        let mut state = RuntimeState::new(&test_endpoint());
        state.worktree_root = Some(temp.join("worktrees"));
        let mut updates = Vec::new();
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::CreateWorkspace {
                root_directory: repository.clone(),
            },
        );
        let parent_workspace_id = state.session.active_workspace_id().unwrap();
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::CreateWorktree {
                parent_workspace_id,
                branch: "feature/server-flow".into(),
            },
        );

        let child_workspace_id = state.session.active_workspace_id().unwrap();
        let child = state.session.workspace(child_workspace_id).unwrap();
        let child_root = child.root_directory().to_path_buf();
        assert!(child.worktree().unwrap().is_managed());
        assert_eq!(
            state
                .workspace_git
                .get(&child_workspace_id)
                .and_then(GitRepository::branch),
            Some("feature/server-flow")
        );

        assert_eq!(
            apply_for_test(
                &mut state,
                &mut updates,
                LayoutCommand::RemoveWorktree {
                    workspace_id: child_workspace_id,
                },
            ),
            1
        );
        assert!(!child_root.exists());
        run_git(
            &repository,
            &["show-ref", "--verify", "refs/heads/feature/server-flow"],
        );

        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::CloseWorkspace {
                workspace_id: parent_workspace_id,
            },
        );
        drop(state);
        drop(updates);
        let _ = std::fs::remove_dir_all(temp);
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn stopping_server_rolls_back_a_prepared_worktree() {
        let temp = std::env::temp_dir().join(format!(
            "murmur-server-worktree-cancel-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let repository = temp.join("repository");
        std::fs::create_dir_all(&repository).unwrap();
        run_git(&repository, &["init"]);
        run_git(&repository, &["config", "user.name", "Murmur Tests"]);
        run_git(
            &repository,
            &["config", "user.email", "murmur@example.invalid"],
        );
        std::fs::write(repository.join("README.md"), "murmur\n").unwrap();
        run_git(&repository, &["add", "README.md"]);
        run_git(&repository, &["commit", "-m", "initial"]);

        let mut state = RuntimeState::new(&test_endpoint());
        state.worktree_root = Some(temp.join("worktrees"));
        state.active_controller = Some(7);
        let parent_workspace_id = state.session.create_workspace(repository.clone());
        let command = LayoutCommand::CreateWorktree {
            parent_workspace_id,
            branch: "feature/cancelled".into(),
        };
        let plan = external_layout_plan(&state, &command).unwrap().unwrap();
        let prepared = prepare_external_layout(plan).unwrap();
        let child_root = match &prepared {
            PreparedExternalLayout::CreateWorktree { child, .. } => child.root().to_path_buf(),
            _ => panic!("expected a prepared worktree"),
        };
        assert!(child_root.exists());
        assert!(matches!(
            layout_authority_error(&state, 7, state.server_id, state.session_id, true),
            Some(ServerMessage::Error { message }) if message == "Server is stopping"
        ));

        cancel_prepared_external_layout(prepared);
        assert!(!child_root.exists());
        run_git(
            &repository,
            &["show-ref", "--verify", "refs/heads/feature/cancelled"],
        );

        let _ = std::fs::remove_dir_all(temp);
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn git_branch_refresh_accepts_activity_from_any_workspace_pane() {
        let temp = std::env::temp_dir().join(format!(
            "murmur-server-branch-refresh-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let repository = temp.join("repository");
        std::fs::create_dir_all(&repository).unwrap();
        run_git(&repository, &["init"]);
        run_git(&repository, &["config", "user.name", "Murmur Tests"]);
        run_git(
            &repository,
            &["config", "user.email", "murmur@example.invalid"],
        );
        std::fs::write(repository.join("README.md"), "murmur\n").unwrap();
        run_git(&repository, &["add", "README.md"]);
        run_git(&repository, &["commit", "-m", "initial"]);

        let mut state = RuntimeState::new(&test_endpoint());
        let workspace_id = state.session.create_workspace(repository.clone());
        let first_pane = state
            .session
            .workspace(workspace_id)
            .unwrap()
            .active_tab()
            .focused_pane()
            .id();
        let second_pane = state
            .session
            .split_pane(first_pane, murmur_core::SplitDirection::Horizontal, 0.5)
            .unwrap();
        let git = discover_repository(&repository).unwrap();
        set_workspace_git(&mut state, workspace_id, git);

        run_git(&repository, &["checkout", "-b", "feature/second-pane"]);
        state.workspace_git_scanned_at.insert(
            workspace_id,
            Instant::now()
                .checked_sub(GIT_SCAN_INTERVAL)
                .expect("test Instant supports subtraction"),
        );
        let activity = Instant::now();
        let WorkspaceGitScan::Ready { workspace_id, root } =
            reserve_workspace_git_scan(&mut state, second_pane, activity, Instant::now())
        else {
            panic!("second Pane activity should reserve its Workspace Git scan");
        };
        let next = discover_repository(&root).unwrap();
        apply_workspace_git_refresh(&mut state, workspace_id, &root, next);
        assert_eq!(
            state
                .workspace_git
                .get(&workspace_id)
                .and_then(GitRepository::branch),
            Some("feature/second-pane")
        );
        assert!(matches!(
            reserve_workspace_git_scan(&mut state, first_pane, activity, Instant::now()),
            WorkspaceGitScan::Covered
        ));

        drop(state);
        let _ = std::fs::remove_dir_all(temp);
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn opening_an_already_open_worktree_records_parent_membership() {
        let temp = std::env::temp_dir().join(format!(
            "murmur-server-open-worktree-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let repository = temp.join("repository");
        let worktree = temp.join("worktree");
        std::fs::create_dir_all(&repository).unwrap();
        run_git(&repository, &["init"]);
        run_git(&repository, &["config", "user.name", "Murmur Tests"]);
        run_git(
            &repository,
            &["config", "user.email", "murmur@example.invalid"],
        );
        std::fs::write(repository.join("README.md"), "murmur\n").unwrap();
        run_git(&repository, &["add", "README.md"]);
        run_git(&repository, &["commit", "-m", "initial"]);
        run_git(
            &repository,
            &[
                "worktree",
                "add",
                "-b",
                "feature/existing",
                worktree.to_str().unwrap(),
            ],
        );

        let mut state = RuntimeState::new(&test_endpoint());
        let parent_workspace_id = state.session.create_workspace(repository.clone());
        let child_workspace_id = state.session.create_workspace(worktree.clone());
        assert!(
            state
                .session
                .workspace(child_workspace_id)
                .unwrap()
                .worktree()
                .is_none()
        );

        let mut updates = Vec::new();
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::OpenWorktree {
                parent_workspace_id,
                root_directory: worktree,
            },
        );
        let association = state
            .session
            .workspace(child_workspace_id)
            .unwrap()
            .worktree()
            .unwrap();
        assert_eq!(association.parent_workspace_id(), parent_workspace_id);
        assert_eq!(association.parent_root_directory(), repository);
        assert!(!association.is_managed());

        drop(state);
        let _ = std::fs::remove_dir_all(temp);
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn pane_terminal_survives_disconnect_and_reconnects_with_live_state() {
        use murmur_core::{TerminalPosition, TerminalScroll, TerminalSide};

        let (handle, endpoint, thread) = start();
        let first_connection = ClientConnection::connect(&endpoint, "first-terminal").unwrap();
        let first_bootstrap = first_connection.bootstrap().clone();
        let server_id = first_bootstrap.server_id;
        let session_id = first_bootstrap.session_id;
        let mut first = first_connection.into_stream();
        first
            .set_handshake_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        murmur_core::protocol::write_message(
            &mut first,
            &ClientMessage::AcquireControl { session_id },
        )
        .unwrap();
        assert!(matches!(
            read_server(&mut first),
            ServerMessage::ControlGranted { .. }
        ));
        murmur_core::protocol::write_message(
            &mut first,
            &ClientMessage::Subscribe {
                session_id,
                after_sequence: first_bootstrap.sequence,
            },
        )
        .unwrap();
        assert!(matches!(
            read_server(&mut first),
            ServerMessage::Subscribed { .. }
        ));
        let mut first_terminal_views = std::collections::HashMap::new();
        murmur_core::protocol::write_message(
            &mut first,
            &ClientMessage::Layout {
                server_id,
                session_id,
                command: LayoutCommand::CreateWorkspace {
                    root_directory: std::env::temp_dir(),
                },
            },
        )
        .unwrap();
        wait_for_message(&mut first, |message| {
            matches!(
                message,
                ServerMessage::Event {
                    event: SessionEvent::LayoutChanged,
                    ..
                }
            )
        });
        let pane_id = handle
            .state
            .lock()
            .unwrap()
            .session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id();
        let pid_command = if cfg!(windows) {
            "Write-Output ('murmur-' + 'pid=' + $PID)\r"
        } else {
            "printf 'murmur-%s=%s\\n' pid $$\r"
        };
        send_terminal(
            &mut first,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Text(pid_command.into()),
        );
        let first_view = wait_for_terminal_text(
            &mut first,
            &mut first_terminal_views,
            pane_id,
            "murmur-pid=",
        );
        let first_pid = marker_value(&view_text(&first_view), "murmur-pid=");
        let snapshot_before_disconnect = handle.snapshot();
        drop(first);
        thread::sleep(Duration::from_millis(30));

        let second_connection = ClientConnection::connect(&endpoint, "second-terminal").unwrap();
        let second_bootstrap = second_connection.bootstrap().clone();
        assert_eq!(second_bootstrap.snapshot, snapshot_before_disconnect);
        let terminal = second_bootstrap
            .terminals
            .iter()
            .find(|terminal| terminal.pane_id == pane_id)
            .expect("reconnect bootstrap contains the live Pane");
        assert!(!terminal.exited);
        assert!(view_text(&terminal.view).contains("murmur-pid="));

        let mut second = second_connection.into_stream();
        second
            .set_handshake_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        murmur_core::protocol::write_message(
            &mut second,
            &ClientMessage::AcquireControl { session_id },
        )
        .unwrap();
        assert!(matches!(
            read_server(&mut second),
            ServerMessage::ControlGranted { .. }
        ));
        murmur_core::protocol::write_message(
            &mut second,
            &ClientMessage::Subscribe {
                session_id,
                after_sequence: second_bootstrap.sequence,
            },
        )
        .unwrap();
        wait_for_message(&mut second, |message| {
            matches!(message, ServerMessage::Subscribed { .. })
        });
        let mut second_terminal_views = second_bootstrap
            .terminals
            .iter()
            .map(|terminal| (terminal.pane_id, terminal.view.clone()))
            .collect();

        let reconnect_command = if cfg!(windows) {
            "Write-Output ('reconnect-' + 'pid=' + $PID)\r"
        } else {
            "printf 'reconnect-%s=%s\\n' pid $$\r"
        };
        send_terminal(
            &mut second,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Text(reconnect_command.into()),
        );
        let reconnect_view = wait_for_terminal_text(
            &mut second,
            &mut second_terminal_views,
            pane_id,
            "reconnect-pid=",
        );
        assert_eq!(
            marker_value(&view_text(&reconnect_view), "reconnect-pid="),
            first_pid
        );

        if cfg!(windows) {
            handle.stop();
            drop(second);
            thread.join().unwrap().unwrap();
            let _ = endpoint.cleanup();
            return;
        }

        send_terminal(
            &mut second,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Paste("printf reconnect-ok\n".into()),
        );
        wait_for_terminal_text(
            &mut second,
            &mut second_terminal_views,
            pane_id,
            "reconnect-ok",
        );
        send_terminal(
            &mut second,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Resize(TerminalSize::new(10, 40)),
        );
        send_terminal(
            &mut second,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Text("stty size\r".into()),
        );
        wait_for_terminal_text(&mut second, &mut second_terminal_views, pane_id, "10 40");
        send_terminal(
            &mut second,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Text(
                "i=1; while [ $i -le 20 ]; do echo line-$i; i=$((i+1)); done\r".into(),
            ),
        );
        wait_for_terminal_text(&mut second, &mut second_terminal_views, pane_id, "line-20");
        thread::sleep(Duration::from_millis(50));
        send_terminal(
            &mut second,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Scroll(TerminalScroll::Top),
        );
        let scrolled =
            wait_for_terminal(&mut second, &mut second_terminal_views, pane_id, |view| {
                view.display_offset > 0
            });
        let expected = (0..4)
            .filter_map(|column| scrolled.cell(0, column))
            .map(|cell| cell.text.as_str())
            .collect::<String>();
        send_terminal(
            &mut second,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Copy {
                selection: murmur_core::TerminalSelection {
                    start: TerminalPosition {
                        row: 0,
                        column: 0,
                        side: TerminalSide::Left,
                    },
                    end: TerminalPosition {
                        row: 0,
                        column: 3,
                        side: TerminalSide::Right,
                    },
                    display_offset: scrolled.display_offset,
                },
            },
        );
        let copied = wait_for_message(
            &mut second,
            |message| matches!(message, ServerMessage::TerminalCopied { pane_id: copied_pane, .. } if *copied_pane == pane_id),
        );
        assert!(matches!(
            copied,
            ServerMessage::TerminalCopied { text: Some(text), .. } if text == expected
        ));

        handle.stop();
        drop(second);
        thread.join().unwrap().unwrap();
        let _ = endpoint.cleanup();
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn send_terminal(
        stream: &mut EndpointStream,
        server_id: ServerId,
        session_id: SessionId,
        pane_id: PaneId,
        command: TerminalCommand,
    ) {
        murmur_core::protocol::write_message(
            stream,
            &ClientMessage::Terminal {
                server_id,
                session_id,
                pane_id,
                command,
            },
        )
        .unwrap();
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn wait_for_terminal_text(
        stream: &mut EndpointStream,
        views: &mut std::collections::HashMap<PaneId, murmur_core::TerminalView>,
        pane_id: PaneId,
        needle: &str,
    ) -> murmur_core::TerminalView {
        wait_for_terminal(stream, views, pane_id, |view| {
            view_text(view).contains(needle)
        })
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn wait_for_terminal(
        stream: &mut EndpointStream,
        views: &mut std::collections::HashMap<PaneId, murmur_core::TerminalView>,
        pane_id: PaneId,
        predicate: impl Fn(&murmur_core::TerminalView) -> bool,
    ) -> murmur_core::TerminalView {
        loop {
            let message = read_server(stream);
            if let ServerMessage::Error { message } = &message {
                panic!("server returned an error: {message}");
            }
            let ServerMessage::TerminalFrame(batch) = message else {
                continue;
            };
            for pane in batch.panes {
                if let Some(view) = views.get_mut(&pane.pane_id) {
                    view.apply_frame(pane.frame).unwrap();
                } else if let murmur_core::TerminalViewFrame::Full(view) = pane.frame {
                    views.insert(pane.pane_id, view);
                } else {
                    panic!("first terminal frame for a Pane must be full");
                }
                let view = &views[&pane.pane_id];
                if pane.pane_id == pane_id && predicate(view) {
                    return view.clone();
                }
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn wait_for_message(
        stream: &mut EndpointStream,
        predicate: impl Fn(&ServerMessage) -> bool,
    ) -> ServerMessage {
        loop {
            let message = read_server(stream);
            if let ServerMessage::Error { message } = &message {
                panic!("server returned an error: {message}");
            }
            if predicate(&message) {
                return message;
            }
        }
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn read_server(stream: &mut EndpointStream) -> ServerMessage {
        murmur_core::protocol::read_message(stream).unwrap()
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn view_text(view: &murmur_core::TerminalView) -> String {
        (0..view.size.rows)
            .map(|row| {
                (0..view.size.columns)
                    .filter_map(|column| view.cell(row, column))
                    .map(|cell| cell.text.as_str())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn marker_value(text: &str, marker: &str) -> String {
        text.split(marker)
            .nth(1)
            .and_then(|tail| tail.split_whitespace().next())
            .expect("terminal marker has a value")
            .to_owned()
    }

    #[test]
    fn compatible_client_gets_bootstrap_and_reconnect_sees_same_epoch() {
        let (handle, endpoint, thread) = start();
        let first = connect_and_bootstrap(&endpoint);
        let first_message: ServerMessage = {
            let mut stream = endpoint.connect().unwrap();
            murmur_core::protocol::write_message(
                &mut stream,
                &ClientMessage::Hello(Hello {
                    version: PROTOCOL_VERSION,
                    client_name: "first".into(),
                }),
            )
            .unwrap();
            murmur_core::protocol::read_message(&mut stream).unwrap()
        };
        let (first_id, first_epoch) = match first_message {
            ServerMessage::Welcome {
                server_id,
                runtime_epoch,
                ..
            } => (server_id, runtime_epoch),
            other => panic!("unexpected message: {other:?}"),
        };
        drop(first);
        let mut second = endpoint.connect().unwrap();
        murmur_core::protocol::write_message(
            &mut second,
            &ClientMessage::Hello(Hello {
                version: PROTOCOL_VERSION,
                client_name: "second".into(),
            }),
        )
        .unwrap();
        let welcome: ServerMessage = murmur_core::protocol::read_message(&mut second).unwrap();
        assert!(matches!(
            welcome,
            ServerMessage::Welcome {
                server_id,
                runtime_epoch,
                error: None,
                ..
            } if server_id == first_id && runtime_epoch == first_epoch
        ));
        handle.stop();
        drop(second);
        thread.join().unwrap().unwrap();
        let _ = endpoint.cleanup();
    }

    #[test]
    fn incompatible_client_is_rejected() {
        let (handle, endpoint, thread) = start();
        let mut stream = endpoint.connect().unwrap();
        murmur_core::protocol::write_message(
            &mut stream,
            &ClientMessage::Hello(Hello {
                version: PROTOCOL_VERSION + 1,
                client_name: "old".into(),
            }),
        )
        .unwrap();
        let response: ServerMessage = murmur_core::protocol::read_message(&mut stream).unwrap();
        assert!(matches!(
            response,
            ServerMessage::Welcome { error: Some(_), .. }
        ));
        handle.stop();
        drop(stream);
        thread.join().unwrap().unwrap();
        let _ = endpoint.cleanup();
    }

    #[test]
    fn oversized_client_frame_is_rejected_with_a_clear_error() {
        let (handle, endpoint, thread) = start();
        let mut stream = connect_and_bootstrap(&endpoint);
        let claimed = (murmur_core::protocol::MAX_FRAME_SIZE as u32) + 1;
        std::io::Write::write_all(&mut stream, &claimed.to_le_bytes()).unwrap();

        let response: ServerMessage = murmur_core::protocol::read_message(&mut stream).unwrap();
        assert!(matches!(
            response,
            ServerMessage::Error { message }
                if message.contains("exceeds maximum")
        ));

        handle.stop();
        drop(stream);
        thread.join().unwrap().unwrap();
        let _ = endpoint.cleanup();
    }

    #[test]
    fn controller_is_exclusive_and_released_on_disconnect() {
        let (handle, endpoint, thread) = start();
        let mut first = connect_and_bootstrap(&endpoint);
        let mut second = connect_and_bootstrap(&endpoint);
        let session_id = handle.state.lock().unwrap().session_id;
        murmur_core::protocol::write_message(
            &mut first,
            &ClientMessage::AcquireControl { session_id },
        )
        .unwrap();
        assert!(matches!(
            murmur_core::protocol::read_message::<_, ServerMessage>(&mut first).unwrap(),
            ServerMessage::ControlGranted { .. }
        ));
        murmur_core::protocol::write_message(
            &mut second,
            &ClientMessage::AcquireControl { session_id },
        )
        .unwrap();
        assert!(matches!(
            murmur_core::protocol::read_message::<_, ServerMessage>(&mut second).unwrap(),
            ServerMessage::ControlDenied { .. }
        ));
        drop(first);
        thread::sleep(Duration::from_millis(20));
        murmur_core::protocol::write_message(
            &mut second,
            &ClientMessage::AcquireControl { session_id },
        )
        .unwrap();
        assert!(matches!(
            murmur_core::protocol::read_message::<_, ServerMessage>(&mut second).unwrap(),
            ServerMessage::ControlGranted { .. }
        ));
        handle.stop();
        drop(second);
        thread.join().unwrap().unwrap();
        let _ = endpoint.cleanup();
    }

    #[test]
    fn snapshot_change_is_replayed_after_the_bootstrap_cursor() {
        let (handle, endpoint, thread) = start();
        let mut first = connect_and_bootstrap(&endpoint);
        let mut second = connect_and_bootstrap(&endpoint);
        let server_id = handle.server_id();
        let session_id = handle.state.lock().unwrap().session_id;

        murmur_core::protocol::write_message(
            &mut first,
            &ClientMessage::AcquireControl { session_id },
        )
        .unwrap();
        assert!(matches!(
            murmur_core::protocol::read_message::<_, ServerMessage>(&mut first).unwrap(),
            ServerMessage::ControlGranted { .. }
        ));
        murmur_core::protocol::write_message(
            &mut first,
            &ClientMessage::Layout {
                server_id,
                session_id,
                command: LayoutCommand::CreateWorkspace {
                    root_directory: std::env::temp_dir(),
                },
            },
        )
        .unwrap();
        assert!(matches!(
            murmur_core::protocol::read_message::<_, ServerMessage>(&mut first).unwrap(),
            ServerMessage::Event {
                server_id: event_server,
                session_id: event_session,
                sequence: 1,
                event: SessionEvent::LayoutChanged,
            } if event_server == server_id && event_session == session_id
        ));
        murmur_core::protocol::write_message(
            &mut first,
            &ClientMessage::ReleaseControl { session_id },
        )
        .unwrap();
        assert!(matches!(
            murmur_core::protocol::read_message::<_, ServerMessage>(&mut first).unwrap(),
            ServerMessage::ControlReleased {
                server_id: released_server,
                session_id: released_session,
            } if released_server == server_id && released_session == session_id
        ));

        murmur_core::protocol::write_message(
            &mut second,
            &ClientMessage::Subscribe {
                session_id,
                after_sequence: 0,
            },
        )
        .unwrap();
        let subscribed_sequence = loop {
            match murmur_core::protocol::read_message::<_, ServerMessage>(&mut second).unwrap() {
                ServerMessage::Event {
                    sequence: 1,
                    event: SessionEvent::LayoutChanged,
                    ..
                } => {}
                ServerMessage::Event { .. } => {}
                ServerMessage::Subscribed {
                    server_id: subscribed_server,
                    session_id: subscribed_session,
                    sequence,
                } => {
                    assert_eq!(subscribed_server, server_id);
                    assert_eq!(subscribed_session, session_id);
                    break sequence;
                }
                other => panic!("unexpected replay response: {other:?}"),
            }
        };
        murmur_core::protocol::write_message(
            &mut second,
            &ClientMessage::SnapshotRequest { session_id },
        )
        .unwrap();
        let bootstrap = loop {
            let message = murmur_core::protocol::read_message(&mut second).unwrap();
            if matches!(message, ServerMessage::Bootstrap(_)) {
                break message;
            }
        };
        assert!(matches!(
            bootstrap,
            ServerMessage::Bootstrap(bootstrap)
                if bootstrap.sequence >= subscribed_sequence
                    && bootstrap.snapshot != Session::new().snapshot()
        ));

        handle.stop();
        drop(first);
        drop(second);
        thread.join().unwrap().unwrap();
        let _ = endpoint.cleanup();
    }

    #[test]
    fn subscribed_client_receives_future_events_in_sequence_order() {
        let (handle, endpoint, thread) = start();
        let mut controller = connect_and_bootstrap(&endpoint);
        let mut subscriber = connect_and_bootstrap(&endpoint);
        let server_id = handle.server_id();
        let session_id = handle.state.lock().unwrap().session_id;

        murmur_core::protocol::write_message(
            &mut subscriber,
            &ClientMessage::Subscribe {
                session_id,
                after_sequence: 0,
            },
        )
        .unwrap();
        assert!(matches!(
            murmur_core::protocol::read_message::<_, ServerMessage>(&mut subscriber).unwrap(),
            ServerMessage::Subscribed { sequence: 0, .. }
        ));

        murmur_core::protocol::write_message(
            &mut controller,
            &ClientMessage::AcquireControl { session_id },
        )
        .unwrap();
        assert!(matches!(
            murmur_core::protocol::read_message::<_, ServerMessage>(&mut controller).unwrap(),
            ServerMessage::ControlGranted { .. }
        ));

        let mut snapshot_sequences = Vec::new();
        for _ in 0..2 {
            murmur_core::protocol::write_message(
                &mut controller,
                &ClientMessage::Layout {
                    server_id,
                    session_id,
                    command: LayoutCommand::CreateWorkspace {
                        root_directory: std::env::temp_dir(),
                    },
                },
            )
            .unwrap();
            loop {
                match murmur_core::protocol::read_message::<_, ServerMessage>(&mut controller)
                    .unwrap()
                {
                    ServerMessage::Event {
                        sequence,
                        event: SessionEvent::LayoutChanged,
                        ..
                    } => {
                        snapshot_sequences.push(sequence);
                        break;
                    }
                    ServerMessage::TerminalFrame(_) => {}
                    other => panic!("unexpected mutation response: {other:?}"),
                }
            }
        }

        let mut previous_sequence = 0;
        let mut replayed_snapshots = Vec::new();
        while replayed_snapshots.len() < snapshot_sequences.len() {
            match murmur_core::protocol::read_message::<_, ServerMessage>(&mut subscriber).unwrap()
            {
                ServerMessage::Event {
                    sequence, event, ..
                } => {
                    assert_eq!(sequence, previous_sequence + 1);
                    previous_sequence = sequence;
                    if event == SessionEvent::LayoutChanged {
                        replayed_snapshots.push(sequence);
                    }
                }
                ServerMessage::TerminalFrame(_) => {}
                other => panic!("unexpected subscriber response: {other:?}"),
            }
        }
        assert_eq!(replayed_snapshots, snapshot_sequences);

        handle.stop();
        drop(controller);
        drop(subscriber);
        thread.join().unwrap().unwrap();
        let _ = endpoint.cleanup();
    }

    #[test]
    fn stop_message_ends_server_and_preserves_session_handle() {
        let (handle, endpoint, thread) = start();
        let mut stream = connect_and_bootstrap(&endpoint);
        murmur_core::protocol::write_message(
            &mut stream,
            &ClientMessage::StopServer {
                server_id: handle.server_id(),
            },
        )
        .unwrap();
        assert_eq!(
            murmur_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap(),
            ServerMessage::ServerStopping
        );
        drop(stream);
        thread.join().unwrap().unwrap();
        assert_eq!(handle.snapshot(), Session::new().snapshot());
        let _ = endpoint.cleanup();
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn stop_message_waits_for_an_inflight_worktree_to_roll_back() {
        let temp = std::env::temp_dir().join(format!(
            "murmur-server-stop-worktree-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let repository = temp.join("repository");
        let marker = temp.join("checkout-started");
        let release = temp.join("release-checkout");
        std::fs::create_dir_all(&repository).unwrap();
        run_git(&repository, &["init"]);
        run_git(&repository, &["config", "user.name", "Murmur Tests"]);
        run_git(
            &repository,
            &["config", "user.email", "murmur@example.invalid"],
        );
        std::fs::write(repository.join("README.md"), "murmur\n").unwrap();
        run_git(&repository, &["add", "README.md"]);
        run_git(&repository, &["commit", "-m", "initial"]);

        let shell_path = |path: &std::path::Path| path.to_string_lossy().replace('\\', "/");
        let hook = repository.join(".git").join("hooks").join("post-checkout");
        std::fs::write(
            &hook,
            format!(
                "#!/bin/sh\nprintf started > '{}'\nwhile [ ! -f '{}' ]; do sleep 0.05; done\n",
                shell_path(&marker),
                shell_path(&release)
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mut permissions = std::fs::metadata(&hook).unwrap().permissions();
            permissions.set_mode(0o755);
            std::fs::set_permissions(&hook, permissions).unwrap();
        }

        let (handle, endpoint, thread) = start();
        handle.state.lock().unwrap().worktree_root = Some(temp.join("worktrees"));
        let server_id = handle.server_id();
        let session_id = handle.state.lock().unwrap().session_id;
        let mut controller = connect_and_bootstrap(&endpoint);
        murmur_core::protocol::write_message(
            &mut controller,
            &ClientMessage::AcquireControl { session_id },
        )
        .unwrap();
        assert!(matches!(
            murmur_core::protocol::read_message::<_, ServerMessage>(&mut controller).unwrap(),
            ServerMessage::ControlGranted { .. }
        ));
        murmur_core::protocol::write_message(
            &mut controller,
            &ClientMessage::Layout {
                server_id,
                session_id,
                command: LayoutCommand::CreateWorkspace {
                    root_directory: repository.clone(),
                },
            },
        )
        .unwrap();
        assert!(matches!(
            murmur_core::protocol::read_message::<_, ServerMessage>(&mut controller).unwrap(),
            ServerMessage::Event {
                event: SessionEvent::LayoutChanged,
                ..
            }
        ));
        let parent_workspace_id = handle
            .state
            .lock()
            .unwrap()
            .session
            .active_workspace_id()
            .unwrap();
        murmur_core::protocol::write_message(
            &mut controller,
            &ClientMessage::Layout {
                server_id,
                session_id,
                command: LayoutCommand::CreateWorktree {
                    parent_workspace_id,
                    branch: "feature/stopping".into(),
                },
            },
        )
        .unwrap();

        let checkout_started = (0..200).any(|_| {
            if marker.exists() {
                true
            } else {
                thread::sleep(Duration::from_millis(10));
                false
            }
        });
        assert!(checkout_started, "Git checkout hook did not start");

        let mut stopper = connect_and_bootstrap(&endpoint);
        murmur_core::protocol::write_message(
            &mut stopper,
            &ClientMessage::StopServer { server_id },
        )
        .unwrap();
        assert_eq!(
            murmur_core::protocol::read_message::<_, ServerMessage>(&mut stopper).unwrap(),
            ServerMessage::ServerStopping
        );
        let stop_signalled = (0..100).any(|_| {
            if handle.stop.load(Ordering::Acquire) {
                true
            } else {
                thread::sleep(Duration::from_millis(5));
                false
            }
        });
        assert!(stop_signalled);
        thread::sleep(Duration::from_millis(50));
        assert!(!thread.is_finished());
        std::fs::write(&release, "release\n").unwrap();
        drop(controller);
        drop(stopper);
        thread.join().unwrap().unwrap();

        let child_root = temp
            .join("worktrees")
            .join("repository")
            .join("feature-stopping");
        assert!(!child_root.exists());
        assert_eq!(handle.state.lock().unwrap().session.workspaces().len(), 1);
        let _ = endpoint.cleanup();
        let _ = std::fs::remove_dir_all(temp);
    }

    #[test]
    fn tcp_endpoint_uses_the_same_handshake_and_bootstrap() {
        let server = BoundServer::bind(ServerConfig {
            endpoint: Endpoint::tcp("127.0.0.1:0".parse().unwrap()),
        })
        .unwrap();
        let handle = server.handle();
        let address = server.local_addr().unwrap().unwrap();
        let endpoint = Endpoint::tcp(address);
        let thread = thread::spawn(move || server.run());
        let mut stream = connect_and_bootstrap(&endpoint);
        murmur_core::protocol::write_message(
            &mut stream,
            &ClientMessage::StopServer {
                server_id: handle.server_id(),
            },
        )
        .unwrap();
        assert_eq!(
            murmur_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap(),
            ServerMessage::ServerStopping
        );
        drop(stream);
        thread.join().unwrap().unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn tcp_reconnect_bootstraps_authoritative_agent_and_git_state() {
        let temp = std::env::temp_dir().join(format!(
            "murmur-server-tcp-state-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let repository = temp.join("repository");
        std::fs::create_dir_all(&repository).unwrap();
        run_git(&repository, &["init"]);
        run_git(&repository, &["config", "user.name", "Murmur Tests"]);
        run_git(
            &repository,
            &["config", "user.email", "murmur@example.invalid"],
        );
        std::fs::write(repository.join("README.md"), "murmur\n").unwrap();
        run_git(&repository, &["add", "README.md"]);
        run_git(&repository, &["commit", "-m", "initial"]);
        run_git(&repository, &["branch", "-M", "main"]);

        let server = BoundServer::bind(ServerConfig {
            endpoint: Endpoint::tcp("127.0.0.1:0".parse().unwrap()),
        })
        .unwrap();
        let handle = server.handle();
        let endpoint = Endpoint::tcp(server.local_addr().unwrap().unwrap());
        let thread = thread::spawn(move || server.run());

        let connection = ClientConnection::connect(&endpoint, "tcp-controller").unwrap();
        let bootstrap = connection.bootstrap().clone();
        let server_id = bootstrap.server_id;
        let session_id = bootstrap.session_id;
        let mut stream = connection.into_stream();
        stream
            .set_handshake_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        murmur_core::protocol::write_message(
            &mut stream,
            &ClientMessage::AcquireControl { session_id },
        )
        .unwrap();
        assert!(matches!(
            read_server(&mut stream),
            ServerMessage::ControlGranted { .. }
        ));
        murmur_core::protocol::write_message(
            &mut stream,
            &ClientMessage::Subscribe {
                session_id,
                after_sequence: bootstrap.sequence,
            },
        )
        .unwrap();
        assert!(matches!(
            read_server(&mut stream),
            ServerMessage::Subscribed { .. }
        ));
        murmur_core::protocol::write_message(
            &mut stream,
            &ClientMessage::Layout {
                server_id,
                session_id,
                command: LayoutCommand::CreateWorkspace {
                    root_directory: repository.clone(),
                },
            },
        )
        .unwrap();
        wait_for_message(&mut stream, |message| {
            matches!(
                message,
                ServerMessage::Event {
                    event: SessionEvent::LayoutChanged,
                    ..
                }
            )
        });
        let (workspace_id, pane_id) = {
            let state = handle.state.lock().unwrap();
            let workspace = state.session.active_workspace().unwrap();
            (workspace.id(), workspace.active_tab().focused_pane().id())
        };
        send_terminal(
            &mut stream,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Text(
                "exec -a codex /bin/bash -c \"echo '◦ Working (1s - esc to interrupt)'; sleep 30 & wait\"\r"
                    .into(),
            ),
        );
        wait_for_message(&mut stream, |message| {
            matches!(
                message,
                ServerMessage::Event {
                    event: SessionEvent::AgentChanged {
                        pane_id: event_pane,
                        agent: Some(AgentSnapshot {
                            kind: murmur_core::AgentKind::Codex,
                            state: murmur_core::AgentState::Working,
                        }),
                    },
                    ..
                } if *event_pane == pane_id
            )
        });

        let reconnect = ClientConnection::connect(&endpoint, "tcp-reconnect").unwrap();
        assert!(reconnect.bootstrap().agents.iter().any(|agent| {
            agent.pane_id == pane_id && agent.agent.kind == murmur_core::AgentKind::Codex
        }));
        assert!(reconnect.bootstrap().workspace_git.iter().any(|git| {
            git.workspace_id == workspace_id && git.branch.as_deref() == Some("main")
        }));

        handle.stop();
        drop(reconnect);
        drop(stream);
        thread.join().unwrap().unwrap();
        let _ = std::fs::remove_dir_all(temp);
    }
}
