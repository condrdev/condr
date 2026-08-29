use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use murmur_core::protocol::{
    BootstrapAssembler, BootstrapBatch, BootstrapHeader, BootstrapRecord, ClientMessage,
    FramingError, Hello, LayoutCommand, MAX_BOOTSTRAP_BATCHES, MAX_BOOTSTRAP_TOTAL_SIZE,
    MAX_CHUNK_PAYLOAD_SIZE, MAX_FRAME_SIZE, PROTOCOL_VERSION, PaneAgentSnapshot, PaneTerminalFrame,
    PaneTerminalSnapshot, RuntimeEpoch, ServerId, ServerMessage, SessionBootstrap, SessionEvent,
    SessionId, TerminalFrameBatch, TerminalFrameChunk, VersionCheck, WorkspaceGitSnapshot,
    check_version, encode_bootstrap_record, encode_pane_terminal_frame,
};
use murmur_core::{
    AgentSnapshot, GitRepository, PaneId, Session, TerminalAgentProbe, TerminalCommand,
    TerminalCwdProbe, TerminalRuntime, TerminalSize, TerminalUpdate, TerminalView,
    TerminalViewFrame, TerminalViewSource, WorkspaceId, create_worktree, default_worktree_root,
    discover_repository, open_worktree, remove_worktree, validate_worktree_removal,
};

use crate::client_writer::{ClientWriteItem, ClientWriter};
use crate::endpoint::{Endpoint, EndpointListener, EndpointStream, default_socket_path};
use crate::persistence::{SnapshotLoad, SnapshotPersistence};

const ACCEPT_POLL: Duration = Duration::from_millis(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(4);
const EVENT_HISTORY_LIMIT: usize = 256;
const AGENT_SCAN_INTERVAL: Duration = Duration::from_millis(500);
const GIT_SCAN_INTERVAL: Duration = Duration::from_secs(2);
const TERMINAL_FRAME_INTERVAL: Duration = Duration::from_micros(16_667);
const BOOTSTRAP_METADATA_HEADROOM: usize = 64 * 1024;
const MAX_PERSISTED_SNAPSHOT_BYTES: usize = MAX_FRAME_SIZE - BOOTSTRAP_METADATA_HEADROOM;

type StartedTerminal = (
    PaneId,
    u64,
    mpsc::Receiver<TerminalUpdate>,
    TerminalAgentProbe,
    TerminalCwdProbe,
    TerminalViewSource,
);
type TerminalBaselines = Vec<(PaneId, Arc<TerminalView>)>;

struct TerminalMonitor {
    pane_id: PaneId,
    instance_id: u64,
    updates: mpsc::Receiver<TerminalUpdate>,
    agent_probe: TerminalAgentProbe,
    cwd_probe: TerminalCwdProbe,
    view_source: TerminalViewSource,
    state: Arc<Mutex<RuntimeState>>,
    lifecycle: Arc<ServerLifecycle>,
}

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub endpoint: Endpoint,
    snapshot_path: Option<PathBuf>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self::new(Endpoint::local(default_socket_path()))
    }
}

impl ServerConfig {
    pub fn new(endpoint: Endpoint) -> Self {
        let snapshot_path = match &endpoint {
            Endpoint::Tcp(address) if address.port() == 0 => None,
            _ => Some(default_snapshot_path(&endpoint)),
        };
        Self {
            endpoint,
            snapshot_path,
        }
    }

    pub fn ephemeral(endpoint: Endpoint) -> Self {
        Self {
            endpoint,
            snapshot_path: None,
        }
    }

    pub fn with_snapshot_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.snapshot_path = Some(path.into());
        self
    }

    pub fn snapshot_path(&self) -> Option<&std::path::Path> {
        self.snapshot_path.as_deref()
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
        let batch_count = bootstrap.batch_count;
        let mut assembler = BootstrapAssembler::new(bootstrap)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        for _ in 0..batch_count {
            let batch = match murmur_core::protocol::read_message(&mut stream)
                .map_err(|error| io::Error::other(error.to_string()))?
            {
                ServerMessage::BootstrapBatch(batch) => batch,
                other => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unexpected Bootstrap batch response: {other:?}"),
                    ));
                }
            };
            assembler
                .push(batch)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        }
        let bootstrap = assembler
            .finish()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
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
    startup_terminals: Vec<StartedTerminal>,
}

impl BoundServer {
    pub fn bind(config: ServerConfig) -> io::Result<Self> {
        let listener = config.endpoint.bind()?;
        listener.set_nonblocking(true)?;
        let (state, startup_terminals) =
            match RuntimeState::recover(&config.endpoint, config.snapshot_path) {
                Ok(restored) => restored,
                Err(error) => {
                    let _ = config.endpoint.cleanup();
                    return Err(error);
                }
            };
        Ok(Self {
            endpoint: config.endpoint,
            listener,
            stop: Arc::new(AtomicBool::new(false)),
            lifecycle: Arc::new(ServerLifecycle::default()),
            state: Arc::new(Mutex::new(state)),
            next_client_id: Arc::new(AtomicU64::new(1)),
            startup_terminals,
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

    fn run_blocking(mut self) -> io::Result<()> {
        for (pane_id, instance_id, updates, agent_probe, cwd_probe, view_source) in
            std::mem::take(&mut self.startup_terminals)
        {
            monitor_terminal(TerminalMonitor {
                pane_id,
                instance_id,
                updates,
                agent_probe,
                cwd_probe,
                view_source,
                state: Arc::clone(&self.state),
                lifecycle: Arc::clone(&self.lifecycle),
            });
        }

        let mut run_result = Ok(());
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
                    run_result = Err(error);
                    break;
                }
            }
        }

        self.lifecycle.begin_stop();
        self.stop.store(true, Ordering::Release);
        self.lifecycle.wait_for_operations();
        let mut terminals = {
            let mut state = self.state.lock().expect("server state lock poisoned");
            state.terminal_instances.clear();
            std::mem::take(&mut state.terminals)
        };
        let mut terminal_result = Ok(());
        let mut final_cwds = Vec::new();
        for (&pane_id, runtime) in &mut terminals {
            let cwd_probe = runtime.cwd_probe();
            let before_close = cwd_probe.observe();
            if let Err(error) = runtime.close() {
                eprintln!(
                    "murmur-server: failed to close Terminal for Pane {}: {error}",
                    pane_id.as_u64()
                );
                if terminal_result.is_ok() {
                    terminal_result = Err(io::Error::other(format!(
                        "failed to close Terminal for Pane {}: {error}",
                        pane_id.as_u64()
                    )));
                }
            }
            if let Some(cwd) = shutdown_cwd(before_close, cwd_probe.observe()) {
                final_cwds.push((pane_id, cwd));
            }
        }
        let mut persistence = {
            let mut state = self.state.lock().expect("server state lock poisoned");
            state.record_terminal_cwds(final_cwds);
            state.persistence.take()
        };
        let persistence_result = persistence
            .as_mut()
            .map_or(Ok(()), SnapshotPersistence::shutdown);
        if let Err(error) = &persistence_result {
            eprintln!("murmur-server: final Session Snapshot flush failed: {error}");
        }
        drop(persistence);
        drop(terminals);
        let cleanup_result = self.endpoint.cleanup();
        if let Err(error) = &cleanup_result {
            eprintln!("murmur-server: endpoint cleanup failed: {error}");
        }
        run_result
            .and(terminal_result)
            .and(persistence_result)
            .and(cleanup_result)
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
    terminal_operations: usize,
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

    fn begin_terminal_operation(self: &Arc<Self>) -> Option<TerminalOperationGuard> {
        let mut state = self.state.lock().expect("server lifecycle lock poisoned");
        if state.stopping {
            return None;
        }
        state.terminal_operations += 1;
        Some(TerminalOperationGuard {
            lifecycle: Arc::clone(self),
        })
    }

    fn is_stopping(&self) -> bool {
        self.state
            .lock()
            .expect("server lifecycle lock poisoned")
            .stopping
    }

    fn wait_for_operations(&self) {
        let state = self.state.lock().expect("server lifecycle lock poisoned");
        let _state = self
            .idle
            .wait_while(state, |state| {
                state.external_operations != 0 || state.terminal_operations != 0
            })
            .expect("server lifecycle lock poisoned");
    }
}

fn shutdown_cwd(
    before_close: (Option<PathBuf>, u64),
    after_close: (Option<PathBuf>, u64),
) -> Option<PathBuf> {
    let persistable = |cwd: Option<PathBuf>| cwd.filter(|cwd| cwd.to_str().is_some());
    if after_close.1 != before_close.1 {
        return persistable(after_close.0).or_else(|| persistable(before_close.0));
    }

    #[cfg(windows)]
    let candidates = [after_close.0, before_close.0];
    #[cfg(not(windows))]
    let candidates = [before_close.0, after_close.0];
    candidates.into_iter().find_map(persistable)
}

fn observe_terminal_cwds(
    probes: Vec<(PaneId, u64, TerminalCwdProbe)>,
) -> Vec<(PaneId, u64, Option<PathBuf>)> {
    probes
        .into_iter()
        .map(|(pane_id, instance_id, probe)| (pane_id, instance_id, probe.cwd()))
        .collect()
}

fn restored_worktree_is_valid(root: &std::path::Path, parent_root: &std::path::Path) -> bool {
    let Ok(Some(parent)) = discover_repository(parent_root) else {
        return false;
    };
    let Ok(child) = open_worktree(&parent, root) else {
        return false;
    };
    let Ok(root) = std::fs::canonicalize(root) else {
        return false;
    };
    let Ok(discovered_root) = std::fs::canonicalize(child.root()) else {
        return false;
    };
    root == discovered_root
}

fn clear_invalid_restored_worktrees(session: &mut Session) -> usize {
    let associations = session
        .workspaces()
        .iter()
        .filter_map(|workspace| {
            workspace.worktree().map(|association| {
                (
                    workspace.id(),
                    workspace.root_directory().to_path_buf(),
                    association.parent_root_directory().to_path_buf(),
                )
            })
        })
        .collect::<Vec<_>>();
    let mut cleared = 0;
    for (workspace_id, root, parent_root) in associations {
        if restored_worktree_is_valid(&root, &parent_root) {
            continue;
        }
        if session.clear_worktree_association(workspace_id) {
            eprintln!(
                "murmur-server: clearing stale worktree association for Workspace {} at {}",
                workspace_id.as_u64(),
                root.display()
            );
            cleared += 1;
        }
    }
    cleared
}

struct ExternalOperationGuard {
    lifecycle: Arc<ServerLifecycle>,
}

struct TerminalOperationGuard {
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

impl Drop for TerminalOperationGuard {
    fn drop(&mut self) {
        let mut state = self
            .lifecycle
            .state
            .lock()
            .expect("server lifecycle lock poisoned");
        state.terminal_operations -= 1;
        if state.external_operations == 0 && state.terminal_operations == 0 {
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
    terminal_instances: std::collections::HashMap<PaneId, u64>,
    next_terminal_instance: u64,
    terminal_views: std::collections::HashMap<PaneId, LatestTerminalView>,
    closing_terminals: std::collections::HashSet<PaneId>,
    exited_terminals: std::collections::HashSet<PaneId>,
    agents: std::collections::HashMap<PaneId, AgentSnapshot>,
    workspace_git: std::collections::HashMap<WorkspaceId, GitRepository>,
    workspace_git_scanned_at: std::collections::HashMap<WorkspaceId, Instant>,
    worktree_root: Option<PathBuf>,
    active_controller: Option<u64>,
    events: std::collections::VecDeque<SequencedEvent>,
    subscribers: std::collections::HashMap<u64, ClientSubscriber>,
    persistence: Option<SnapshotPersistence>,
}

struct ClientSubscriber {
    writer: ClientWriter,
    terminal_baselines: std::collections::HashMap<PaneId, Arc<TerminalView>>,
    pending_terminals: std::collections::HashSet<PaneId>,
    render_generation: u64,
    bootstrap_pending: bool,
    deferred_reliable: std::collections::VecDeque<Vec<u8>>,
}

struct BootstrapCapture {
    server_id: ServerId,
    runtime_epoch: RuntimeEpoch,
    session_id: SessionId,
    sequence: u64,
    snapshot: murmur_core::SessionSnapshot,
    terminals: Vec<BootstrapTerminalCapture>,
    agents: Vec<PaneAgentSnapshot>,
    workspace_git: Vec<WorkspaceGitSnapshot>,
    zoomed_panes: Vec<PaneId>,
}

struct BootstrapTerminalCapture {
    pane_id: PaneId,
    view: BootstrapTerminalView,
    exited: bool,
}

enum BootstrapTerminalView {
    Live(TerminalViewSource),
    Retained(Arc<TerminalView>),
}

impl BootstrapTerminalView {
    fn materialize(self) -> TerminalView {
        match self {
            Self::Live(source) => source.view(),
            Self::Retained(view) => Arc::unwrap_or_clone(view),
        }
    }
}

impl BootstrapCapture {
    fn materialize(self) -> SessionBootstrap {
        SessionBootstrap {
            server_id: self.server_id,
            runtime_epoch: self.runtime_epoch,
            session_id: self.session_id,
            sequence: self.sequence,
            snapshot: self.snapshot,
            terminals: self
                .terminals
                .into_iter()
                .map(|terminal| PaneTerminalSnapshot {
                    pane_id: terminal.pane_id,
                    view: terminal.view.materialize(),
                    exited: terminal.exited,
                })
                .collect(),
            agents: self.agents,
            workspace_git: self.workspace_git,
            zoomed_panes: self.zoomed_panes,
        }
    }
}

struct LatestTerminalView {
    view: Arc<TerminalView>,
    producer_frame: Option<TerminalViewFrame>,
}

struct TerminalRenderSnapshot {
    client_id: u64,
    generation: u64,
    server_id: ServerId,
    session_id: SessionId,
    panes: Vec<TerminalRenderPaneSnapshot>,
}

struct TerminalRenderPaneSnapshot {
    pane_id: PaneId,
    baseline: Option<Arc<TerminalView>>,
    current: Arc<TerminalView>,
    producer_frame: Option<TerminalViewFrame>,
}

struct PreparedTerminalRender {
    client_id: u64,
    generation: u64,
    server_id: ServerId,
    session_id: SessionId,
    pending: Vec<PaneId>,
    baselines: Vec<(PaneId, Arc<TerminalView>)>,
    frames: Vec<PaneTerminalFrame>,
}

enum TerminalRenderCommit {
    Done,
    Retry,
}

impl ClientSubscriber {
    fn try_send_terminal_render(
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

struct SequencedEvent {
    sequence: u64,
    event: SessionEvent,
}

impl RuntimeState {
    #[cfg(test)]
    fn new(endpoint: &Endpoint) -> Self {
        Self::with_session(endpoint, Session::new(), None)
    }

    fn with_session(
        endpoint: &Endpoint,
        session: Session,
        persistence: Option<SnapshotPersistence>,
    ) -> Self {
        Self {
            server_id: ServerId(stable_endpoint_id(endpoint)),
            runtime_epoch: RuntimeEpoch(runtime_epoch()),
            session_id: SessionId(1),
            sequence: 0,
            session,
            terminals: std::collections::HashMap::new(),
            terminal_instances: std::collections::HashMap::new(),
            next_terminal_instance: 1,
            terminal_views: std::collections::HashMap::new(),
            closing_terminals: std::collections::HashSet::new(),
            exited_terminals: std::collections::HashSet::new(),
            agents: std::collections::HashMap::new(),
            workspace_git: std::collections::HashMap::new(),
            workspace_git_scanned_at: std::collections::HashMap::new(),
            worktree_root: default_worktree_root().ok(),
            active_controller: None,
            events: std::collections::VecDeque::new(),
            subscribers: std::collections::HashMap::new(),
            persistence,
        }
    }

    fn recover(
        endpoint: &Endpoint,
        snapshot_path: Option<PathBuf>,
    ) -> io::Result<(Self, Vec<StartedTerminal>)> {
        let persistence = snapshot_path.map(SnapshotPersistence::open).transpose()?;
        let mut session = Session::new();
        let mut restored = false;
        if let Some(persistence) = persistence.as_ref() {
            match persistence.load() {
                SnapshotLoad::Missing => {}
                SnapshotLoad::Loaded(snapshot) => match validate_persistable_snapshot(&snapshot) {
                    Ok(()) => match Session::restore(snapshot) {
                        Ok(loaded) => {
                            session = loaded;
                            restored = true;
                        }
                        Err(error) => eprintln!(
                            "murmur-server: ignoring invalid Session Snapshot at {}: {error}",
                            persistence.path().display()
                        ),
                    },
                    Err(error) => eprintln!(
                        "murmur-server: ignoring invalid Session Snapshot at {}: {error}",
                        persistence.path().display()
                    ),
                },
                SnapshotLoad::Rejected(reason) => eprintln!(
                    "murmur-server: ignoring invalid Session Snapshot at {}: {reason}",
                    persistence.path().display()
                ),
            }
        }

        let repaired_worktrees = if restored {
            clear_invalid_restored_worktrees(&mut session)
        } else {
            0
        };

        let launch_specs = session
            .workspaces()
            .iter()
            .flat_map(|workspace| {
                workspace.tabs().iter().flat_map(move |tab| {
                    tab.panes().iter().map(move |pane| {
                        (
                            pane.id(),
                            pane.cwd().map(PathBuf::from),
                            workspace.root_directory().to_path_buf(),
                        )
                    })
                })
            })
            .collect::<Vec<_>>();
        let mut started_runtimes = Vec::new();
        let mut failed_panes = Vec::new();
        let mut repaired_cwds = 0;
        for (pane_id, saved_cwd, workspace_root) in launch_specs {
            let requested_cwd = saved_cwd.as_deref().unwrap_or(workspace_root.as_path());
            let started = match TerminalRuntime::spawn_shell(
                requested_cwd,
                TerminalSize::new(24, 80),
            ) {
                Ok(runtime) => Some((runtime, requested_cwd.to_path_buf())),
                Err(error) if requested_cwd != workspace_root.as_path() => {
                    eprintln!(
                        "murmur-server: fresh shell failed for Pane {} in saved cwd {}; retrying Workspace Root {}: {error}",
                        pane_id.as_u64(),
                        requested_cwd.display(),
                        workspace_root.display()
                    );
                    match TerminalRuntime::spawn_shell(&workspace_root, TerminalSize::new(24, 80)) {
                        Ok(runtime) => Some((runtime, workspace_root)),
                        Err(fallback_error) => {
                            eprintln!(
                                "murmur-server: pruning Pane {} after fresh shell also failed in Workspace Root: {fallback_error}",
                                pane_id.as_u64()
                            );
                            None
                        }
                    }
                }
                Err(error) => {
                    eprintln!(
                        "murmur-server: pruning Pane {} after fresh shell failed in {}: {error}",
                        pane_id.as_u64(),
                        requested_cwd.display()
                    );
                    None
                }
            };
            let Some((mut runtime, started_cwd)) = started else {
                failed_panes.push(pane_id);
                continue;
            };
            if saved_cwd.is_some()
                && saved_cwd.as_deref() != Some(started_cwd.as_path())
                && session.set_pane_cwd(pane_id, Some(started_cwd))
            {
                repaired_cwds += 1;
            }
            let updates = runtime
                .take_updates()
                .expect("new Terminal update receiver exists");
            started_runtimes.push((pane_id, runtime, updates));
        }
        for pane_id in &failed_panes {
            session
                .close_pane(*pane_id)
                .expect("restored Pane remains until it is pruned");
        }

        let mut state = Self::with_session(endpoint, session, persistence);
        let workspace_roots = state
            .session
            .workspaces()
            .iter()
            .map(|workspace| (workspace.id(), workspace.root_directory().to_path_buf()))
            .collect::<Vec<_>>();
        for (workspace_id, root) in workspace_roots {
            set_workspace_git(
                &mut state,
                workspace_id,
                discover_repository(root).ok().flatten(),
            );
        }
        let startup_terminals = started_runtimes
            .into_iter()
            .map(|(pane_id, runtime, updates)| state.install_terminal(pane_id, runtime, updates))
            .collect();
        if restored && (!failed_panes.is_empty() || repaired_worktrees != 0 || repaired_cwds != 0) {
            state.schedule_snapshot(state.session.snapshot());
        }
        Ok((state, startup_terminals))
    }

    fn install_terminal(
        &mut self,
        pane_id: PaneId,
        runtime: TerminalRuntime,
        updates: mpsc::Receiver<TerminalUpdate>,
    ) -> StartedTerminal {
        let instance_id = self.next_terminal_instance;
        self.next_terminal_instance = self.next_terminal_instance.wrapping_add(1).max(1);
        let probe = runtime
            .agent_probe()
            .expect("new Terminal has an agent probe");
        let cwd_probe = runtime.cwd_probe();
        let view_source = runtime.view_source();
        self.terminal_views.insert(
            pane_id,
            LatestTerminalView {
                view: Arc::new(runtime.view()),
                producer_frame: None,
            },
        );
        for subscriber in self.subscribers.values_mut() {
            subscriber.terminal_baselines.remove(&pane_id);
            subscriber.pending_terminals.insert(pane_id);
            subscriber.render_generation = subscriber.render_generation.wrapping_add(1);
        }
        self.terminals.insert(pane_id, runtime);
        self.terminal_instances.insert(pane_id, instance_id);
        self.closing_terminals.remove(&pane_id);
        (pane_id, instance_id, updates, probe, cwd_probe, view_source)
    }

    fn restore_exited_terminal(&mut self, pane_id: PaneId, runtime: TerminalRuntime) -> bool {
        if self.session.pane(pane_id).is_none() || self.terminals.contains_key(&pane_id) {
            return false;
        }
        let view = runtime.view();
        self.terminal_instances.remove(&pane_id);
        self.closing_terminals.remove(&pane_id);
        self.terminals.insert(pane_id, runtime);
        self.publish_terminal(pane_id, TerminalViewFrame::Full(view));
        if self.exited_terminals.insert(pane_id) {
            self.publish_background(SessionEvent::TerminalExited { pane_id });
        }
        if self.agents.remove(&pane_id).is_some() {
            self.publish_background(SessionEvent::AgentChanged {
                pane_id,
                agent: None,
            });
        }
        true
    }

    fn terminal_is_current(&self, pane_id: PaneId, instance_id: u64) -> bool {
        self.terminal_instances.get(&pane_id) == Some(&instance_id)
    }

    fn schedule_snapshot(&self, snapshot: murmur_core::SessionSnapshot) {
        if let Some(persistence) = &self.persistence {
            persistence.schedule(snapshot);
        }
    }

    fn record_terminal_cwds(&mut self, cwds: impl IntoIterator<Item = (PaneId, PathBuf)>) -> bool {
        let mut candidate = self.session.clone();
        let mut updates = Vec::new();
        for (pane_id, cwd) in cwds {
            let Some(pane) = candidate.pane(pane_id) else {
                continue;
            };
            if pane.cwd() == Some(cwd.as_path()) {
                continue;
            }
            if cwd.to_str().is_none() {
                eprintln!(
                    "murmur-server: ignoring unpersistable Terminal cwd update for Pane {}: path is not valid UTF-8",
                    pane_id.as_u64()
                );
                continue;
            }
            candidate.set_pane_cwd(pane_id, Some(cwd.clone()));
            updates.push((pane_id, cwd));
        }
        if updates.is_empty() {
            return false;
        }

        let snapshot = candidate.snapshot();
        if validate_persistable_snapshot(&snapshot).is_ok() {
            self.session = candidate;
            self.schedule_snapshot(snapshot);
            return true;
        }

        candidate = self.session.clone();
        let mut accepted = false;
        let mut accepted_snapshot = None;
        for (pane_id, cwd) in updates {
            let previous = candidate
                .pane(pane_id)
                .and_then(|pane| pane.cwd())
                .map(PathBuf::from);
            candidate.set_pane_cwd(pane_id, Some(cwd));
            let snapshot = candidate.snapshot();
            match validate_persistable_snapshot(&snapshot) {
                Ok(()) => {
                    accepted = true;
                    accepted_snapshot = Some(snapshot);
                }
                Err(error) => {
                    candidate.set_pane_cwd(pane_id, previous);
                    eprintln!(
                        "murmur-server: ignoring unpersistable Terminal cwd update for Pane {}: {error}",
                        pane_id.as_u64()
                    );
                }
            }
        }
        if accepted {
            self.session = candidate;
            self.schedule_snapshot(
                accepted_snapshot.expect("an accepted cwd update produced a Snapshot"),
            );
            true
        } else {
            false
        }
    }

    fn terminal_cwd_probes(&self) -> Vec<(PaneId, u64, TerminalCwdProbe)> {
        self.terminals
            .iter()
            .filter(|(pane_id, _)| !self.exited_terminals.contains(pane_id))
            .filter(|(pane_id, _)| !self.closing_terminals.contains(pane_id))
            .filter_map(|(&pane_id, runtime)| {
                self.terminal_instances
                    .get(&pane_id)
                    .copied()
                    .map(|instance_id| (pane_id, instance_id, runtime.cwd_probe()))
            })
            .collect()
    }

    fn terminal_cwd_probe(&self, pane_id: PaneId) -> Option<(PaneId, u64, TerminalCwdProbe)> {
        if self.exited_terminals.contains(&pane_id) || self.closing_terminals.contains(&pane_id) {
            return None;
        }
        Some((
            pane_id,
            *self.terminal_instances.get(&pane_id)?,
            self.terminals.get(&pane_id)?.cwd_probe(),
        ))
    }

    fn terminal_cwd_probes_for_layout(
        &self,
        command: &LayoutCommand,
    ) -> Vec<(PaneId, u64, TerminalCwdProbe)> {
        if !layout_command_needs_cwd_observation(command) {
            return Vec::new();
        }
        let pane_id = match command {
            LayoutCommand::CreateTab { workspace_id } => self
                .session
                .workspace(*workspace_id)
                .map(|workspace| workspace.active_tab().focused_pane().id()),
            LayoutCommand::SplitPane { pane_id, .. } => Some(*pane_id),
            _ => None,
        };
        pane_id
            .and_then(|pane_id| self.terminal_cwd_probe(pane_id))
            .into_iter()
            .collect()
    }

    fn terminal_cwd_probes_for_workspace(
        &self,
        workspace_id: WorkspaceId,
    ) -> Vec<(PaneId, u64, TerminalCwdProbe)> {
        self.session
            .workspace(workspace_id)
            .into_iter()
            .flat_map(|workspace| workspace.tabs())
            .flat_map(|tab| tab.panes())
            .filter_map(|pane| self.terminal_cwd_probe(pane.id()))
            .collect()
    }

    fn record_terminal_cwd_observations(
        &mut self,
        observations: Vec<(PaneId, u64, Option<PathBuf>)>,
    ) -> bool {
        let cwds = observations
            .into_iter()
            .filter_map(|(pane_id, instance_id, cwd)| {
                (self.terminal_is_current(pane_id, instance_id)
                    && !self.closing_terminals.contains(&pane_id)
                    && !self.exited_terminals.contains(&pane_id))
                .then_some(cwd)
                .flatten()
                .map(|cwd| (pane_id, cwd))
            })
            .collect::<Vec<_>>();
        self.record_terminal_cwds(cwds)
    }

    fn capture_bootstrap(&self) -> BootstrapCapture {
        let mut terminals = Vec::new();
        for workspace in self.session.workspaces() {
            for tab in workspace.tabs() {
                for pane in tab.panes() {
                    if let Some(runtime) = self.terminals.get(&pane.id()) {
                        terminals.push(BootstrapTerminalCapture {
                            pane_id: pane.id(),
                            view: BootstrapTerminalView::Live(runtime.view_source()),
                            exited: self.exited_terminals.contains(&pane.id()),
                        });
                    } else if self.closing_terminals.contains(&pane.id())
                        && let Some(latest) = self.terminal_views.get(&pane.id())
                    {
                        terminals.push(BootstrapTerminalCapture {
                            pane_id: pane.id(),
                            view: BootstrapTerminalView::Retained(Arc::clone(&latest.view)),
                            exited: self.exited_terminals.contains(&pane.id()),
                        });
                    }
                }
            }
        }
        BootstrapCapture {
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

    #[cfg(test)]
    fn bootstrap(&self) -> SessionBootstrap {
        self.capture_bootstrap().materialize()
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
            } else if subscriber.bootstrap_pending {
                subscriber.deferred_reliable.push_back(data.clone());
                true
            } else {
                subscriber.writer.send_reliable(data.clone()).is_ok()
            }
        });
        origin_failed
    }

    fn publish_terminal(&mut self, pane_id: PaneId, frame: TerminalViewFrame) -> Option<Vec<u64>> {
        let latest = match frame {
            TerminalViewFrame::Full(view) => LatestTerminalView {
                view: Arc::new(view),
                producer_frame: None,
            },
            TerminalViewFrame::Delta(delta) => {
                let latest = self.terminal_views.get_mut(&pane_id)?;
                Arc::make_mut(&mut latest.view)
                    .apply_frame(TerminalViewFrame::Delta(delta.clone()))
                    .ok()?;
                latest.producer_frame = Some(TerminalViewFrame::Delta(delta));
                let client_ids = self.subscribers.keys().copied().collect::<Vec<_>>();
                for subscriber in self.subscribers.values_mut() {
                    subscriber.pending_terminals.insert(pane_id);
                    subscriber.render_generation = subscriber.render_generation.wrapping_add(1);
                }
                return Some(client_ids);
            }
        };
        self.terminal_views.insert(pane_id, latest);
        let client_ids = self.subscribers.keys().copied().collect::<Vec<_>>();
        for subscriber in self.subscribers.values_mut() {
            subscriber.pending_terminals.insert(pane_id);
            subscriber.render_generation = subscriber.render_generation.wrapping_add(1);
        }
        Some(client_ids)
    }

    fn terminal_render_snapshot(&mut self, client_id: u64) -> Option<TerminalRenderSnapshot> {
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

    fn begin_bootstrap(&mut self, client_id: u64) {
        let Some(subscriber) = self.subscribers.get_mut(&client_id) else {
            return;
        };
        subscriber.writer.clear_render();
        subscriber.pending_terminals.clear();
        subscriber.terminal_baselines.clear();
        subscriber.bootstrap_pending = true;
        subscriber.deferred_reliable.clear();
        subscriber.render_generation = subscriber.render_generation.wrapping_add(1);
    }

    fn ensure_subscriber(&mut self, client_id: u64, writer: ClientWriter) -> bool {
        match self.subscribers.entry(client_id) {
            std::collections::hash_map::Entry::Occupied(_) => false,
            std::collections::hash_map::Entry::Vacant(entry) => {
                let pending_terminals = self.terminals.keys().copied().collect();
                entry.insert(ClientSubscriber {
                    writer,
                    terminal_baselines: std::collections::HashMap::new(),
                    pending_terminals,
                    render_generation: 1,
                    bootstrap_pending: false,
                    deferred_reliable: std::collections::VecDeque::new(),
                });
                true
            }
        }
    }

    fn finish_bootstrap(&mut self, client_id: u64, framed: FramedBootstrap) -> bool {
        let Some(subscriber) = self.subscribers.get_mut(&client_id) else {
            return true;
        };
        subscriber.writer.clear_render();
        subscriber.terminal_baselines = framed.baselines.into_iter().collect();
        if subscriber
            .writer
            .send_reliable_batch(framed.frames)
            .is_err()
        {
            return true;
        }
        while let Some(data) = subscriber.deferred_reliable.pop_front() {
            if subscriber.writer.send_reliable(data).is_err() {
                return true;
            }
        }
        subscriber.bootstrap_pending = false;
        subscriber.render_generation = subscriber.render_generation.wrapping_add(1);
        false
    }

    fn abort_bootstrap(&mut self, client_id: u64) {
        if let Some(subscriber) = self.subscribers.get_mut(&client_id) {
            subscriber.bootstrap_pending = false;
            subscriber.deferred_reliable.clear();
            subscriber.pending_terminals.clear();
            subscriber.render_generation = subscriber.render_generation.wrapping_add(1);
        }
    }
}

struct LayoutEffect {
    started_terminals: Vec<StartedTerminal>,
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
    },
}

struct TerminalRestartSpec {
    pane_id: PaneId,
    cwd: PathBuf,
    size: TerminalSize,
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
        runtime: Box<TerminalRuntime>,
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
        restart_specs: Vec<TerminalRestartSpec>,
        stopped_terminals: Vec<(PaneId, TerminalRuntime)>,
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
            let mut runtime = match TerminalRuntime::spawn_shell(
                child.root(),
                TerminalSize::new(24, 80),
            ) {
                Ok(runtime) => runtime,
                Err(error) => {
                    return match remove_worktree(&parent, &child) {
                        Ok(()) => Err(format!("failed to start terminal: {error}")),
                        Err(cleanup_error) => Err(format!(
                            "failed to start terminal: {error}; failed to remove the prepared worktree: {cleanup_error}"
                        )),
                    };
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
                runtime: Box::new(runtime),
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
                restart_specs: Vec::new(),
                stopped_terminals: Vec::new(),
            })
        }
    }
}

fn approve_external_layout(
    state: &mut RuntimeState,
    prepared: &mut PreparedExternalLayout,
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
            restart_specs,
            stopped_terminals,
            ..
        } => {
            let workspace = state
                .session
                .workspace(*workspace_id)
                .filter(|workspace| {
                    workspace.root_directory() == child.root()
                        && workspace
                            .worktree()
                            .is_some_and(|association| association.is_managed())
                })
                .ok_or_else(|| "managed Workspace changed while preparing removal".to_string())?;
            let root = workspace.root_directory().to_path_buf();
            let pane_ids = workspace
                .tabs()
                .iter()
                .flat_map(|tab| tab.panes())
                .map(|pane| pane.id())
                .collect::<Vec<_>>();
            for pane_id in pane_ids {
                if let Some(runtime) = state.terminals.remove(&pane_id) {
                    if !state.exited_terminals.contains(&pane_id) {
                        let cwd = state
                            .session
                            .pane(pane_id)
                            .and_then(|pane| pane.cwd())
                            .unwrap_or(&root)
                            .to_path_buf();
                        restart_specs.push(TerminalRestartSpec {
                            pane_id,
                            cwd,
                            size: runtime.size(),
                        });
                    }
                    state.terminal_instances.remove(&pane_id);
                    state.closing_terminals.insert(pane_id);
                    stopped_terminals.push((pane_id, runtime));
                }
            }
        }
        _ => {}
    }
    Ok(())
}

struct ExternalLayoutFinishError {
    message: String,
    restarted: Vec<(PaneId, TerminalRuntime, mpsc::Receiver<TerminalUpdate>)>,
    exited: Vec<(PaneId, TerminalRuntime)>,
    cwds: Vec<(PaneId, PathBuf)>,
}

fn finish_external_layout(
    prepared: PreparedExternalLayout,
) -> Result<PreparedExternalLayout, ExternalLayoutFinishError> {
    match prepared {
        PreparedExternalLayout::RemoveWorktreeReady {
            workspace_id,
            parent,
            child,
            restart_specs,
            mut stopped_terminals,
            ..
        } => {
            let mut close_errors = Vec::new();
            let mut final_cwds = std::collections::HashMap::new();
            for (pane_id, runtime) in &mut stopped_terminals {
                let cwd_probe = runtime.cwd_probe();
                let before_close = cwd_probe.observe();
                if let Err(error) = runtime.close() {
                    close_errors.push(format!("Pane {}: {error}", pane_id.as_u64()));
                }
                if let Some(cwd) = shutdown_cwd(before_close, cwd_probe.observe()) {
                    final_cwds.insert(*pane_id, cwd);
                }
            }
            let removal = if close_errors.is_empty() {
                remove_worktree(&parent, &child).map_err(|error| error.to_string())
            } else {
                Err(format!(
                    "failed to close terminals before removing the worktree: {}",
                    close_errors.join(", ")
                ))
            };
            match removal {
                Ok(()) => Ok(PreparedExternalLayout::RemoveWorktree { workspace_id }),
                Err(mut message) => {
                    let mut restarted = Vec::new();
                    let mut exited = Vec::new();
                    let mut restart_errors = Vec::new();
                    for spec in restart_specs {
                        let cwd = final_cwds.get(&spec.pane_id).cloned().unwrap_or(spec.cwd);
                        match TerminalRuntime::spawn_shell(&cwd, spec.size) {
                            Ok(mut runtime) => {
                                let updates = runtime
                                    .take_updates()
                                    .expect("new Terminal update receiver exists");
                                restarted.push((spec.pane_id, runtime, updates));
                                if let Some(index) = stopped_terminals
                                    .iter()
                                    .position(|(pane_id, _)| *pane_id == spec.pane_id)
                                {
                                    stopped_terminals.swap_remove(index);
                                }
                            }
                            Err(restart_error) => {
                                restart_errors.push(format!(
                                    "Pane {} in {}: {restart_error}",
                                    spec.pane_id.as_u64(),
                                    cwd.display()
                                ));
                                if let Some(index) = stopped_terminals
                                    .iter()
                                    .position(|(pane_id, _)| *pane_id == spec.pane_id)
                                {
                                    exited.push(stopped_terminals.swap_remove(index));
                                }
                            }
                        }
                    }
                    exited.extend(stopped_terminals);
                    if !restart_errors.is_empty() {
                        message.push_str("; failed to restart terminals after removal failed: ");
                        message.push_str(&restart_errors.join(", "));
                    }
                    Err(ExternalLayoutFinishError {
                        message,
                        restarted,
                        exited,
                        cwds: final_cwds.into_iter().collect(),
                    })
                }
            }
        }
        prepared => Ok(prepared),
    }
}

fn cancel_prepared_external_layout(prepared: PreparedExternalLayout) -> Result<(), String> {
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
        remove_worktree(&parent, &child).map_err(|error| {
            format!("failed to remove the prepared worktree during rollback: {error}")
        })?;
    }
    Ok(())
}

fn prepared_worktree_failure(
    parent: &GitRepository,
    child: &GitRepository,
    message: String,
) -> String {
    match remove_worktree(parent, child) {
        Ok(()) => message,
        Err(error) => format!("{message}; failed to remove the prepared worktree: {error}"),
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
            let workspace_id = candidate
                .create_workspace(root_directory)
                .ok_or_else(|| "Session Workspace limit reached".to_string())?;
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
                .ok_or_else(|| "unknown Workspace or Session Tab limit reached".to_string())?;
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
                    .ok_or_else(|| {
                        "unknown Pane or Session Pane/layout depth limit reached".to_string()
                    })?,
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

    commit_layout_candidate(state, candidate, closed, started)
}

fn layout_command_needs_cwd_observation(command: &LayoutCommand) -> bool {
    matches!(
        command,
        LayoutCommand::CreateTab { .. } | LayoutCommand::SplitPane { .. }
    )
}

fn commit_layout_candidate(
    state: &mut RuntimeState,
    candidate: Session,
    closed: Option<murmur_core::CloseOutcome>,
    started: Option<(PaneId, TerminalRuntime, mpsc::Receiver<TerminalUpdate>)>,
) -> Result<LayoutEffect, String> {
    let next_snapshot = candidate.snapshot();
    validate_persistable_snapshot(&next_snapshot)?;
    let durable_changed = next_snapshot != state.session.snapshot();
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
            state.closing_terminals.remove(pane_id);
            state.agents.remove(pane_id);
            state.terminal_views.remove(pane_id);
            state.terminal_instances.remove(pane_id);
            if let Some(runtime) = state.terminals.remove(pane_id) {
                removed_terminals.push(runtime);
            }
        }
    }
    let started_terminals = started
        .map(|(pane_id, runtime, updates)| state.install_terminal(pane_id, runtime, updates));
    if durable_changed {
        state.schedule_snapshot(next_snapshot);
    }
    Ok(LayoutEffect {
        started_terminals: started_terminals.into_iter().collect(),
        removed_terminals,
    })
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
            parent,
            child,
            runtime,
            updates,
        } => {
            let mut candidate = state.session.clone();
            if candidate
                .workspace(parent_workspace_id)
                .is_none_or(|workspace| workspace.root_directory() != parent_root)
            {
                drop(updates);
                drop(runtime);
                return Err(prepared_worktree_failure(
                    &parent,
                    &child,
                    "parent Workspace changed while creating its worktree".into(),
                ));
            }
            let Some(workspace_id) = candidate.create_workspace(child.root().to_path_buf()) else {
                drop(updates);
                drop(runtime);
                return Err(prepared_worktree_failure(
                    &parent,
                    &child,
                    "Session Workspace limit reached".into(),
                ));
            };
            if !candidate.associate_worktree(workspace_id, parent_workspace_id, parent_root, true) {
                drop(updates);
                drop(runtime);
                return Err(prepared_worktree_failure(
                    &parent,
                    &child,
                    "failed to associate the created worktree".into(),
                ));
            }
            let pane_id = candidate
                .workspace(workspace_id)
                .expect("created Workspace exists")
                .active_tab()
                .focused_pane()
                .id();
            let snapshot = candidate.snapshot();
            if let Err(error) = validate_persistable_snapshot(&snapshot) {
                drop(updates);
                drop(runtime);
                return Err(prepared_worktree_failure(&parent, &child, error));
            }
            let effect = match commit_layout_candidate(
                state,
                candidate,
                None,
                Some((pane_id, *runtime, updates)),
            ) {
                Ok(effect) => effect,
                Err(error) => {
                    return Err(prepared_worktree_failure(&parent, &child, error));
                }
            };
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
                let workspace_id = candidate
                    .create_workspace(child.root().to_path_buf())
                    .ok_or_else(|| "Session Workspace limit reached".to_string())?;
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
            let effect = commit_layout_candidate(state, candidate, None, started)?;
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
            commit_layout_candidate(state, candidate, Some(closed), None)
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
            let result = match item {
                ClientWriteItem::Reliable(data) => send_framed(&mut writer_stream, &data),
                ClientWriteItem::ReliableBatch(frames) => frames
                    .into_iter()
                    .try_for_each(|data| send_framed(&mut writer_stream, &data)),
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
        let mut started_terminals = Vec::new();
        let mut removed_terminals = Vec::new();
        let should_close = match message {
            ClientMessage::SnapshotRequest { session_id } => {
                queue_runtime_bootstrap(&state, client_id, session_id, &outbound)
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
                let failed = responses
                    .into_iter()
                    .any(|response| queue_message(&outbound, response));
                if failed {
                    state.subscribers.remove(&client_id);
                }
                let should_flush = should_subscribe && !failed;
                drop(state);
                if should_flush {
                    flush_terminal_render(&shared_state, client_id);
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
                        Ok(mut prepared) => {
                            let probes = {
                                let state = state.lock().expect("server state lock poisoned");
                                if let Some(message) = layout_authority_error(
                                    &state,
                                    client_id,
                                    server_id,
                                    session_id,
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
                                    lifecycle.is_stopping(),
                                ) {
                                    Err(Box::new(message))
                                } else {
                                    state.record_terminal_cwd_observations(observations);
                                    approve_external_layout(&mut state, &mut prepared).map_err(
                                        |message| Box::new(ServerMessage::Error { message }),
                                    )
                                }
                            });
                            match approval {
                                Err(message) => {
                                    let failed = queue_message(&outbound, *message);
                                    let cleanup_failed = cancel_prepared_external_layout(prepared)
                                        .is_err_and(|message| {
                                            eprintln!("murmur-server: {message}");
                                            queue_message(
                                                &outbound,
                                                ServerMessage::Error { message },
                                            )
                                        });
                                    failed || cleanup_failed
                                }
                                Ok(()) => match finish_external_layout(prepared) {
                                    Err(failure) => {
                                        let ExternalLayoutFinishError {
                                            message,
                                            restarted,
                                            exited,
                                            cwds,
                                        } = failure;
                                        let stopping = lifecycle.is_stopping();
                                        let clients = {
                                            let mut state =
                                                state.lock().expect("server state lock poisoned");
                                            state.record_terminal_cwds(cwds);
                                            if stopping {
                                                Vec::new()
                                            } else {
                                                let mut cleared_agents = Vec::new();
                                                for (pane_id, runtime, updates) in restarted {
                                                    if state.session.pane(pane_id).is_none()
                                                        || state.terminals.contains_key(&pane_id)
                                                    {
                                                        continue;
                                                    }
                                                    state.exited_terminals.remove(&pane_id);
                                                    if state.agents.remove(&pane_id).is_some() {
                                                        cleared_agents.push(pane_id);
                                                    }
                                                    started_terminals
                                                        .extend([state.install_terminal(
                                                            pane_id, runtime, updates,
                                                        )]);
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
                                                    state.restore_exited_terminal(pane_id, runtime);
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
                                            let failed = queue_message(&outbound, message);
                                            let cleanup_failed =
                                                cancel_prepared_external_layout(prepared)
                                                    .is_err_and(|message| {
                                                        eprintln!("murmur-server: {message}");
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
                    let probes = {
                        let state = state.lock().expect("server state lock poisoned");
                        if let Some(message) = layout_authority_error(
                            &state,
                            client_id,
                            server_id,
                            session_id,
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
                                lifecycle.is_stopping(),
                            ) {
                                queue_message(&outbound, message)
                            } else {
                                state.record_terminal_cwd_observations(observations);
                                match apply_layout_command(&mut state, command) {
                                    Ok(effect) => {
                                        started_terminals.extend(effect.started_terminals);
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
                } else if lifecycle.is_stopping() {
                    queue_message(
                        &outbound,
                        ServerMessage::Error {
                            message: "Server is stopping".into(),
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
                } else if state.closing_terminals.contains(&pane_id) {
                    queue_message(
                        &outbound,
                        ServerMessage::Error {
                            message: "terminal is closing".into(),
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
        for (pane_id, instance_id, updates, agent_probe, cwd_probe, view_source) in
            started_terminals
        {
            monitor_terminal(TerminalMonitor {
                pane_id,
                instance_id,
                updates,
                agent_probe,
                cwd_probe,
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
    state.subscribers.remove(&client_id);
    if state.active_controller == Some(client_id) {
        state.active_controller = None;
    }
    drop(state);
    if stopping_server {
        stop.store(true, Ordering::Release);
    }
    drop(outbound);
    let _ = writer.join();
}

fn monitor_terminal(monitor: TerminalMonitor) {
    let TerminalMonitor {
        pane_id,
        instance_id,
        updates,
        agent_probe,
        cwd_probe,
        view_source,
        state,
        lifecycle,
    } = monitor;
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
                    let Some(frame) = view_source.take_frame() else {
                        continue;
                    };
                    if !publish_terminal_frame(&state, &view_source, pane_id, instance_id, frame) {
                        break;
                    }
                    last_view_publish = Instant::now();
                }
                Some(TerminalUpdate::Exited) => {
                    let Some(_operation) = lifecycle.begin_terminal_operation() else {
                        break;
                    };
                    let cwd_before_shutdown = cwd_probe.observe();
                    let runtime = {
                        let mut state = state.lock().expect("server state lock poisoned");
                        if !state.terminal_is_current(pane_id, instance_id) {
                            None
                        } else {
                            state.terminal_instances.remove(&pane_id);
                            state.closing_terminals.insert(pane_id);
                            state.terminals.remove(&pane_id)
                        }
                    };
                    let Some(mut runtime) = runtime else {
                        break;
                    };
                    if let Err(error) = runtime.close() {
                        eprintln!(
                            "murmur-server: failed to reap Terminal for Pane {} after PTY EOF: {error}",
                            pane_id.as_u64()
                        );
                    }
                    let cwd = shutdown_cwd(cwd_before_shutdown, cwd_probe.observe());
                    let final_view = view_source.view();
                    let clients = {
                        let mut state = state.lock().expect("server state lock poisoned");
                        if state.session.pane(pane_id).is_none()
                            || state.terminals.contains_key(&pane_id)
                        {
                            break;
                        }
                        state.terminals.insert(pane_id, runtime);
                        state.terminal_instances.insert(pane_id, instance_id);
                        state.closing_terminals.remove(&pane_id);
                        let clients = state
                            .publish_terminal(pane_id, TerminalViewFrame::Full(final_view))
                            .unwrap_or_default();
                        if let Some(cwd) = cwd {
                            state.record_terminal_cwds([(pane_id, cwd)]);
                        }
                        state.exited_terminals.insert(pane_id);
                        state.publish_background(SessionEvent::TerminalExited { pane_id });
                        if state.agents.remove(&pane_id).is_some() {
                            state.publish_background(SessionEvent::AgentChanged {
                                pane_id,
                                agent: None,
                            });
                        }
                        clients
                    };
                    for client_id in clients {
                        flush_terminal_render(&state, client_id);
                    }
                    break;
                }
                None => {}
            }

            let now = Instant::now();
            if agent_scan_pending && now.duration_since(last_agent_scan) >= AGENT_SCAN_INTERVAL {
                let previous = {
                    let state = state.lock().expect("server state lock poisoned");
                    if !state.terminal_is_current(pane_id, instance_id) {
                        break;
                    }
                    state.agents.get(&pane_id).copied()
                };
                let next = agent_probe.snapshot(previous);
                let cwd = cwd_probe.cwd();
                let mut state = state.lock().expect("server state lock poisoned");
                if !state.terminal_is_current(pane_id, instance_id) {
                    break;
                }
                if let Some(cwd) = cwd {
                    state.record_terminal_cwds([(pane_id, cwd)]);
                }
                apply_agent_refresh(&mut state, pane_id, next);
                last_agent_scan = now;
                agent_scan_pending = false;
            }
            if let Some(activity) = git_scan_pending {
                let scan = {
                    let mut state = state.lock().expect("server state lock poisoned");
                    if !state.terminal_is_current(pane_id, instance_id) {
                        break;
                    }
                    reserve_workspace_git_scan(&mut state, pane_id, activity, now)
                };
                match scan {
                    WorkspaceGitScan::Waiting => {}
                    WorkspaceGitScan::Covered => git_scan_pending = None,
                    WorkspaceGitScan::Ready { workspace_id, root } => {
                        git_scan_pending = None;
                        if let Ok(next) = discover_repository(&root) {
                            let mut state = state.lock().expect("server state lock poisoned");
                            if !state.terminal_is_current(pane_id, instance_id) {
                                break;
                            }
                            apply_workspace_git_refresh(&mut state, workspace_id, &root, next);
                        }
                    }
                }
            }
        }
    });
}

fn publish_terminal_frame(
    state: &Arc<Mutex<RuntimeState>>,
    view_source: &TerminalViewSource,
    pane_id: PaneId,
    instance_id: u64,
    frame: TerminalViewFrame,
) -> bool {
    let clients = {
        let mut state = state.lock().expect("server state lock poisoned");
        if !state.terminal_is_current(pane_id, instance_id) {
            return false;
        }
        state.publish_terminal(pane_id, frame)
    };
    let clients = match clients {
        Some(clients) => clients,
        None => {
            let full = TerminalViewFrame::Full(view_source.view());
            let mut state = state.lock().expect("server state lock poisoned");
            if !state.terminal_is_current(pane_id, instance_id) {
                return false;
            }
            state
                .publish_terminal(pane_id, full)
                .expect("a full terminal frame establishes a retained view")
        }
    };
    for client_id in clients {
        flush_terminal_render(state, client_id);
    }
    true
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

fn queue_runtime_bootstrap(
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
                ServerMessage::Error {
                    message: "unknown Session".into(),
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
                ServerMessage::Error {
                    message: "unknown Session".into(),
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
                ServerMessage::Error {
                    message: format!("cannot encode Session Bootstrap: {error}"),
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

fn flush_terminal_render(state: &Arc<Mutex<RuntimeState>>, client_id: u64) {
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

fn prepare_terminal_render(snapshot: TerminalRenderSnapshot) -> PreparedTerminalRender {
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

fn frame_message(message: &ServerMessage) -> io::Result<Vec<u8>> {
    let mut data = Vec::new();
    murmur_core::protocol::write_message(&mut data, message)
        .map_err(|error| io::Error::other(error.to_string()))?;
    Ok(data)
}

fn validate_persistable_snapshot(snapshot: &murmur_core::SessionSnapshot) -> Result<(), String> {
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

struct FramedBootstrap {
    frames: Vec<Vec<u8>>,
    baselines: TerminalBaselines,
}

fn frame_bootstrap_messages(bootstrap: SessionBootstrap) -> io::Result<FramedBootstrap> {
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

fn split_bootstrap_records(
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

fn frame_terminal_batches(
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

fn send_framed(stream: &mut EndpointStream, data: &[u8]) -> io::Result<()> {
    stream.write_all(data)?;
    stream.flush()
}

fn default_snapshot_path(endpoint: &Endpoint) -> PathBuf {
    if let Some(path) = std::env::var_os("MURMUR_SNAPSHOT_PATH")
        && !path.is_empty()
    {
        return PathBuf::from(path);
    }
    snapshot_path_for_endpoint(endpoint)
}

fn snapshot_path_for_endpoint(endpoint: &Endpoint) -> PathBuf {
    match endpoint {
        Endpoint::Local(path) => {
            let mut snapshot = path.as_os_str().to_os_string();
            snapshot.push(".snapshot");
            PathBuf::from(snapshot)
        }
        Endpoint::Tcp(_) => default_socket_path().with_file_name(format!(
            "murmur-server-{:016x}.snapshot",
            stable_endpoint_id(endpoint)
        )),
    }
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
    fn snapshot_paths_preserve_the_complete_local_endpoint_name() {
        let socket = snapshot_path_for_endpoint(&Endpoint::local("murmur.sock"));
        let pipe = snapshot_path_for_endpoint(&Endpoint::local("murmur.pipe"));

        assert_eq!(socket, PathBuf::from("murmur.sock.snapshot"));
        assert_eq!(pipe, PathBuf::from("murmur.pipe.snapshot"));
        assert_ne!(socket, pipe);
    }

    #[test]
    fn an_os_assigned_tcp_port_is_ephemeral_without_an_explicit_snapshot() {
        let endpoint = Endpoint::tcp("127.0.0.1:0".parse().unwrap());

        assert!(
            ServerConfig::new(endpoint.clone())
                .snapshot_path()
                .is_none()
        );
        assert_eq!(
            ServerConfig::new(endpoint)
                .with_snapshot_path("explicit.snapshot")
                .snapshot_path(),
            Some(std::path::Path::new("explicit.snapshot"))
        );
    }

    #[test]
    fn only_cwd_inheriting_layout_commands_require_process_observation() {
        let mut session = Session::new();
        let workspace_id = session
            .create_workspace(std::env::temp_dir())
            .expect("Workspace capacity");
        let tab_id = session.active_workspace().unwrap().active_tab().id();
        let pane_id = session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id();

        assert!(layout_command_needs_cwd_observation(
            &LayoutCommand::CreateTab { workspace_id }
        ));
        assert!(layout_command_needs_cwd_observation(
            &LayoutCommand::SplitPane {
                pane_id,
                direction: murmur_core::SplitDirection::Horizontal,
            }
        ));
        assert!(!layout_command_needs_cwd_observation(
            &LayoutCommand::SetSplitRatios {
                tab_id,
                ratios: Vec::new(),
            }
        ));
        assert!(!layout_command_needs_cwd_observation(
            &LayoutCommand::FocusPane { pane_id }
        ));
    }

    #[test]
    fn shutdown_cwd_uses_a_report_committed_while_the_reader_is_joining() {
        let before = PathBuf::from("before");
        let after = PathBuf::from("after");

        assert_eq!(
            shutdown_cwd((Some(before), 4), (Some(after.clone()), 5)),
            Some(after)
        );
    }

    #[test]
    fn shutdown_cwd_keeps_the_platform_probe_priority_without_a_new_report() {
        let before = PathBuf::from("before");
        let after = PathBuf::from("after");
        let selected = shutdown_cwd((Some(before.clone()), 4), (Some(after.clone()), 4));

        #[cfg(windows)]
        assert_eq!(selected, Some(after));
        #[cfg(not(windows))]
        assert_eq!(selected, Some(before));
    }

    #[cfg(unix)]
    #[test]
    fn shutdown_cwd_rejects_a_non_utf8_tail_report() {
        use std::os::unix::ffi::OsStringExt as _;

        let before = PathBuf::from("before");
        let invalid = PathBuf::from(std::ffi::OsString::from_vec(vec![0xff]));

        assert_eq!(
            shutdown_cwd((Some(before.clone()), 4), (Some(invalid), 5)),
            Some(before)
        );
    }

    fn terminal_test_view(revision: u64, text: &str) -> TerminalView {
        let cells = text
            .chars()
            .map(|character| murmur_core::TerminalCell {
                text: character.to_string().into(),
                foreground: murmur_core::TerminalColor::Named(0),
                background: murmur_core::TerminalColor::Named(0),
                flags: 0,
            })
            .collect::<Vec<_>>();
        TerminalView {
            revision,
            size: TerminalSize::new(1, u16::try_from(cells.len()).unwrap()),
            display_offset: 0,
            cells,
            cursor: None,
        }
    }

    #[test]
    fn bootstrap_retains_a_closing_terminal_without_inventing_an_exit() {
        let mut state = RuntimeState::new(&test_endpoint());
        state
            .session
            .create_workspace(std::env::temp_dir())
            .expect("Workspace capacity");
        let pane_id = state
            .session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id();
        state.terminal_views.insert(
            pane_id,
            LatestTerminalView {
                view: Arc::new(terminal_test_view(1, "tail")),
                producer_frame: None,
            },
        );
        state.closing_terminals.insert(pane_id);

        let bootstrap = state.bootstrap();
        assert_eq!(bootstrap.terminals.len(), 1);
        assert!(!bootstrap.terminals[0].exited);
        assert_eq!(view_text(&bootstrap.terminals[0].view), "tail");

        state.exited_terminals.insert(pane_id);
        assert!(state.bootstrap().terminals[0].exited);
    }

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
        session
            .create_workspace(std::env::temp_dir())
            .expect("Workspace capacity");
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
            render_generation: 0,
            bootstrap_pending: false,
            deferred_reliable: std::collections::VecDeque::new(),
        };

        subscriber
            .try_send_terminal_render(vec![vec![1]], vec![(pane_id, Arc::new(view(1)))])
            .unwrap();
        assert_eq!(subscriber.terminal_baselines[&pane_id].revision, 1);
        assert!(matches!(
            subscriber.try_send_terminal_render(vec![vec![2]], vec![(pane_id, Arc::new(view(2)))]),
            Err(mpsc::TrySendError::Full(_))
        ));
        assert_eq!(subscriber.terminal_baselines[&pane_id].revision, 1);

        assert_eq!(
            receiver.recv(),
            Some(ClientWriteItem::Render {
                data: vec![1],
                slot_drained: true,
            })
        );
        subscriber
            .try_send_terminal_render(vec![vec![3]], vec![(pane_id, Arc::new(view(3)))])
            .unwrap();
        assert_eq!(subscriber.terminal_baselines[&pane_id].revision, 3);
    }

    #[test]
    fn bootstrap_fence_queues_concurrent_events_after_the_complete_bootstrap() {
        let endpoint = test_endpoint();
        let mut state = RuntimeState::new(&endpoint);
        let capture = state.capture_bootstrap();
        let (writer, receiver) = ClientWriter::channel();
        state.subscribers.insert(
            7,
            ClientSubscriber {
                writer,
                terminal_baselines: std::collections::HashMap::new(),
                pending_terminals: std::collections::HashSet::new(),
                render_generation: 0,
                bootstrap_pending: false,
                deferred_reliable: std::collections::VecDeque::new(),
            },
        );

        state.begin_bootstrap(7);
        state.publish_background(SessionEvent::LayoutChanged);
        assert!(state.terminal_render_snapshot(7).is_none());
        assert_eq!(state.subscribers[&7].deferred_reliable.len(), 1);

        let framed = frame_bootstrap_messages(capture.materialize()).unwrap();
        assert!(!state.finish_bootstrap(7, framed));
        let Some(ClientWriteItem::ReliableBatch(bootstrap_frames)) = receiver.recv() else {
            panic!("complete Bootstrap must be the first reliable item");
        };
        assert!(!bootstrap_frames.is_empty());
        let Some(ClientWriteItem::Reliable(event_frame)) = receiver.recv() else {
            panic!("event raised during Bootstrap must follow its batch");
        };
        let event =
            murmur_core::protocol::read_message::<_, ServerMessage>(&mut event_frame.as_slice())
                .unwrap();
        assert!(matches!(
            event,
            ServerMessage::Event {
                event: SessionEvent::LayoutChanged,
                ..
            }
        ));
        assert!(!state.subscribers[&7].bootstrap_pending);
    }

    #[test]
    fn successful_resubscribe_preserves_the_committed_terminal_baseline() {
        let endpoint = test_endpoint();
        let mut state = RuntimeState::new(&endpoint);
        state
            .session
            .create_workspace(std::env::temp_dir())
            .expect("Workspace capacity");
        let pane_id = state
            .session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id();
        let baseline = Arc::new(terminal_test_view(7, "baseline"));
        let (writer, _receiver) = ClientWriter::channel();
        state.subscribers.insert(
            7,
            ClientSubscriber {
                writer,
                terminal_baselines: std::collections::HashMap::from([(
                    pane_id,
                    Arc::clone(&baseline),
                )]),
                pending_terminals: std::collections::HashSet::new(),
                render_generation: 11,
                bootstrap_pending: false,
                deferred_reliable: std::collections::VecDeque::new(),
            },
        );
        let (replacement_writer, _replacement_receiver) = ClientWriter::channel();

        assert!(!state.ensure_subscriber(7, replacement_writer));

        let subscriber = &state.subscribers[&7];
        assert_eq!(subscriber.terminal_baselines[&pane_id].revision, 7);
        assert!(subscriber.pending_terminals.is_empty());
        assert_eq!(subscriber.render_generation, 11);
    }

    #[test]
    fn full_render_slot_regenerates_the_latest_tail_after_drain() {
        let endpoint = test_endpoint();
        let mut state = RuntimeState::new(&endpoint);
        state
            .session
            .create_workspace(std::env::temp_dir())
            .expect("Workspace capacity");
        let pane_id = state
            .session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id();
        let initial = Arc::new(terminal_test_view(1, "old"));
        let latest = Arc::new(terminal_test_view(3, "oXd"));
        state.terminal_views.insert(
            pane_id,
            LatestTerminalView {
                view: Arc::clone(&latest),
                producer_frame: None,
            },
        );
        let (writer, receiver) = ClientWriter::channel();
        writer.try_send_render(vec![vec![0]]).unwrap();
        state.subscribers.insert(
            7,
            ClientSubscriber {
                writer,
                terminal_baselines: std::collections::HashMap::from([(
                    pane_id,
                    Arc::clone(&initial),
                )]),
                pending_terminals: std::collections::HashSet::from([pane_id]),
                render_generation: 1,
                bootstrap_pending: false,
                deferred_reliable: std::collections::VecDeque::new(),
            },
        );
        let state = Arc::new(Mutex::new(state));

        flush_terminal_render(&state, 7);
        {
            let state = state.lock().unwrap();
            let subscriber = &state.subscribers[&7];
            assert_eq!(subscriber.terminal_baselines[&pane_id].revision, 1);
            assert!(subscriber.pending_terminals.contains(&pane_id));
        }

        assert_eq!(
            receiver.recv(),
            Some(ClientWriteItem::Render {
                data: vec![0],
                slot_drained: true,
            })
        );
        flush_terminal_render(&state, 7);
        let Some(ClientWriteItem::Render { data, .. }) = receiver.recv() else {
            panic!("latest terminal tail was not regenerated");
        };
        let message: ServerMessage =
            murmur_core::protocol::read_message(&mut data.as_slice()).unwrap();
        assert!(matches!(
            message,
            ServerMessage::TerminalFrame(TerminalFrameBatch { panes, .. })
                if panes.len() == 1
                    && matches!(
                        &panes[0].frame,
                        TerminalViewFrame::Delta(delta)
                            if delta.base_revision == 1 && delta.revision == 3
                    )
        ));
        let state = state.lock().unwrap();
        let subscriber = &state.subscribers[&7];
        assert_eq!(subscriber.terminal_baselines[&pane_id].revision, 3);
        assert!(!subscriber.pending_terminals.contains(&pane_id));
    }

    #[test]
    fn terminal_batches_are_split_before_the_protocol_limit() {
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
        let large_view = |revision| TerminalView {
            revision,
            size: TerminalSize::new(1, 1),
            display_offset: 0,
            cells: vec![murmur_core::TerminalCell {
                text: "x".repeat(MAX_FRAME_SIZE / 2 + 1024).into(),
                foreground: murmur_core::TerminalColor::Named(0),
                background: murmur_core::TerminalColor::Named(0),
                flags: 0,
            }],
            cursor: None,
        };
        let frames = frame_terminal_batches(
            ServerId(1),
            SessionId(1),
            vec![
                PaneTerminalFrame {
                    pane_id,
                    frame: TerminalViewFrame::Full(large_view(1)),
                },
                PaneTerminalFrame {
                    pane_id,
                    frame: TerminalViewFrame::Full(large_view(2)),
                },
            ],
        )
        .unwrap();
        assert_eq!(frames.len(), 2);
        for (revision, frame) in [1, 2].into_iter().zip(frames) {
            assert!(frame.len() <= MAX_FRAME_SIZE + size_of::<u32>());
            let mut frame = frame.as_slice();
            let message: ServerMessage = murmur_core::protocol::read_message(&mut frame).unwrap();
            assert!(matches!(
                message,
                ServerMessage::TerminalFrame(TerminalFrameBatch { panes, .. })
                    if panes.len() == 1
                        && matches!(
                            &panes[0].frame,
                            TerminalViewFrame::Full(view) if view.revision == revision
                        )
            ));
            assert!(frame.is_empty());
        }
    }

    #[test]
    fn oversized_terminal_frame_is_transported_as_ordered_chunks() {
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
        let expected = PaneTerminalFrame {
            pane_id,
            frame: TerminalViewFrame::Full(TerminalView {
                revision: 9,
                size: TerminalSize::new(1, 1),
                display_offset: 0,
                cells: vec![murmur_core::TerminalCell {
                    text: "x".repeat(MAX_CHUNK_PAYLOAD_SIZE + 1_024).into(),
                    foreground: murmur_core::TerminalColor::Named(0),
                    background: murmur_core::TerminalColor::Named(0),
                    flags: 0,
                }],
                cursor: None,
            }),
        };
        let frames = frame_terminal_batches(ServerId(1), SessionId(1), vec![expected.clone()])
            .expect("oversized terminal frame should be chunked");
        let mut payload = Vec::new();
        let mut chunks = 0;
        for frame in frames {
            assert!(frame.len() <= MAX_FRAME_SIZE + size_of::<u32>());
            let mut frame = frame.as_slice();
            let message: ServerMessage = murmur_core::protocol::read_message(&mut frame).unwrap();
            let ServerMessage::TerminalFrameChunk(chunk) = message else {
                panic!("oversized terminal frame should use chunk messages");
            };
            assert_eq!(chunk.pane_id, pane_id);
            assert_eq!(chunk.revision, 9);
            assert_eq!(chunk.chunk_index, chunks);
            chunks += 1;
            assert_eq!(chunk.chunk_count, 2);
            payload.extend(chunk.payload);
            assert!(frame.is_empty());
        }
        assert_eq!(chunks, 2);
        assert_eq!(
            murmur_core::protocol::decode_pane_terminal_frame(&payload).unwrap(),
            expected
        );
    }

    #[test]
    fn bootstrap_dynamic_records_are_split_and_reassembled() {
        let mut session = Session::new();
        session
            .create_workspace(std::env::temp_dir())
            .expect("Workspace capacity");
        let first_pane = session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id();
        let second_pane = session
            .split_pane(first_pane, murmur_core::SplitDirection::Horizontal, 0.5)
            .unwrap();
        let large_view = |revision| TerminalView {
            revision,
            size: TerminalSize::new(1, 1),
            display_offset: 0,
            cells: vec![murmur_core::TerminalCell {
                text: "x".repeat(MAX_CHUNK_PAYLOAD_SIZE + 1_024).into(),
                foreground: murmur_core::TerminalColor::Named(0),
                background: murmur_core::TerminalColor::Named(0),
                flags: 0,
            }],
            cursor: None,
        };
        let expected = SessionBootstrap {
            server_id: ServerId(1),
            runtime_epoch: RuntimeEpoch(2),
            session_id: SessionId(1),
            sequence: 0,
            snapshot: session.snapshot(),
            terminals: vec![
                PaneTerminalSnapshot {
                    pane_id: first_pane,
                    view: large_view(1),
                    exited: false,
                },
                PaneTerminalSnapshot {
                    pane_id: second_pane,
                    view: large_view(2),
                    exited: true,
                },
            ],
            agents: Vec::new(),
            workspace_git: vec![WorkspaceGitSnapshot {
                workspace_id: session.active_workspace_id().unwrap(),
                branch: Some("b".repeat(MAX_CHUNK_PAYLOAD_SIZE + 1_024)),
                linked_worktree: false,
            }],
            zoomed_panes: vec![first_pane],
        };
        let frames = frame_bootstrap_messages(expected.clone()).unwrap().frames;

        assert!(frames.len() >= 7, "three large records should be chunked");
        assert!(frames.iter().all(|frame| frame.len() <= MAX_FRAME_SIZE + 4));
        let header: ServerMessage =
            murmur_core::protocol::read_message(&mut frames[0].as_slice()).unwrap();
        let ServerMessage::Bootstrap(header) = header else {
            panic!("first frame should be a Bootstrap header");
        };
        assert_eq!(header.batch_count as usize, frames.len() - 1);
        assert_eq!(header.snapshot, expected.snapshot);
        let mut assembler = BootstrapAssembler::new(header).unwrap();
        for frame in &frames[1..] {
            let message: ServerMessage =
                murmur_core::protocol::read_message(&mut frame.as_slice()).unwrap();
            let ServerMessage::BootstrapBatch(batch) = message else {
                panic!("Bootstrap payload should contain only batch frames");
            };
            assembler.push(batch).unwrap();
        }
        assert_eq!(assembler.finish().unwrap(), expected);
    }

    #[test]
    fn untransportable_durable_mutations_leave_the_session_unchanged() {
        let endpoint = test_endpoint();
        let mut state = RuntimeState::new(&endpoint);
        let workspace_id = state
            .session
            .create_workspace(std::env::temp_dir())
            .expect("Workspace capacity");
        let pane_id = state
            .session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id();
        let before = state.session.snapshot();

        let error = apply_layout_command(
            &mut state,
            LayoutCommand::RenameWorkspace {
                workspace_id,
                name: "x".repeat(MAX_PERSISTED_SNAPSHOT_BYTES),
            },
        )
        .err()
        .expect("oversized durable mutation should fail");
        assert!(error.contains("Server limit"));
        assert_eq!(state.session.snapshot(), before);

        assert!(!state.record_terminal_cwds([(
            pane_id,
            PathBuf::from("x".repeat(MAX_PERSISTED_SNAPSHOT_BYTES))
        )]));
        assert_eq!(state.session.snapshot(), before);
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_terminal_cwd_does_not_poison_the_durable_snapshot() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt as _;

        let endpoint = test_endpoint();
        let mut state = RuntimeState::new(&endpoint);
        state
            .session
            .create_workspace(std::env::temp_dir())
            .expect("Workspace capacity");
        let pane_id = state
            .session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id();
        let before = state.session.snapshot();
        let cwd = PathBuf::from(OsString::from_vec(vec![b'/', b't', b'm', b'p', b'/', 0xff]));

        assert!(!state.record_terminal_cwds([(pane_id, cwd)]));
        assert_eq!(state.session.snapshot(), before);
        assert!(state.session.snapshot().to_bytes().is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn invalid_terminal_cwd_does_not_discard_valid_sibling_observations() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt as _;

        let endpoint = test_endpoint();
        let mut state = RuntimeState::new(&endpoint);
        state
            .session
            .create_workspace(std::env::temp_dir())
            .expect("Workspace capacity");
        let first_pane = state
            .session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id();
        let second_pane = state
            .session
            .split_pane(first_pane, murmur_core::SplitDirection::Horizontal, 0.5)
            .unwrap();
        let first_before = state
            .session
            .pane(first_pane)
            .and_then(|pane| pane.cwd())
            .map(PathBuf::from);
        let valid = std::env::temp_dir().join("murmur-valid-cwd");
        let invalid = PathBuf::from(OsString::from_vec(vec![b'/', b't', b'm', b'p', b'/', 0xff]));

        assert!(state.record_terminal_cwds([(first_pane, invalid), (second_pane, valid.clone()),]));
        assert_eq!(
            state
                .session
                .pane(first_pane)
                .and_then(|pane| pane.cwd())
                .map(PathBuf::from),
            first_before
        );
        assert_eq!(
            state.session.pane(second_pane).and_then(|pane| pane.cwd()),
            Some(valid.as_path())
        );
        assert!(state.session.snapshot().to_bytes().is_ok());
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn stale_cwd_observation_cannot_update_a_replaced_terminal() {
        let directory = std::env::temp_dir().join(format!(
            "murmur-server-stale-cwd-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let initial = directory.join("initial");
        let stale = directory.join("stale");
        std::fs::create_dir_all(&initial).unwrap();
        std::fs::create_dir_all(&stale).unwrap();

        let endpoint = test_endpoint();
        let mut state = RuntimeState::new(&endpoint);
        state
            .session
            .create_workspace(initial.clone())
            .expect("Workspace capacity");
        let pane_id = state
            .session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id();
        let mut runtime =
            TerminalRuntime::spawn_shell(&initial, TerminalSize::new(24, 80)).unwrap();
        let updates = runtime.take_updates().unwrap();
        let (_, stale_instance, ..) = state.install_terminal(pane_id, runtime, updates);
        state
            .terminal_instances
            .insert(pane_id, stale_instance.wrapping_add(1));

        assert!(!state.record_terminal_cwd_observations(vec![(
            pane_id,
            stale_instance,
            Some(stale),
        )]));
        assert_eq!(
            state.session.pane(pane_id).and_then(|pane| pane.cwd()),
            Some(initial.as_path())
        );

        state.terminal_instances.insert(pane_id, stale_instance);
        state.exited_terminals.insert(pane_id);
        assert!(!state.record_terminal_cwd_observations(vec![(
            pane_id,
            stale_instance,
            Some(directory.join("exited-stale")),
        )]));
        assert_eq!(
            state.session.pane(pane_id).and_then(|pane| pane.cwd()),
            Some(initial.as_path())
        );

        drop(state);
        let _ = std::fs::remove_dir_all(directory);
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
        let server = BoundServer::bind(ServerConfig::ephemeral(endpoint.clone())).unwrap();
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

    fn persist_snapshot(path: PathBuf, snapshot: murmur_core::SessionSnapshot) {
        let mut persistence =
            SnapshotPersistence::open_with_debounce(path, Duration::from_millis(10)).unwrap();
        persistence.schedule(snapshot);
        persistence.shutdown().unwrap();
    }

    fn structurally_invalid_snapshot(root: PathBuf) -> Vec<u8> {
        let mut session = Session::new();
        let workspace_id = session
            .create_workspace(root.clone())
            .expect("Workspace capacity");
        let tab = session.active_workspace().unwrap().active_tab();
        let tab_id = tab.id();
        let pane_id = tab.focused_pane().id();
        bincode::serialize(&(
            1u32,
            vec![(
                workspace_id,
                "invalid".to_string(),
                root.clone(),
                Option::<murmur_core::WorktreeAssociation>::None,
                vec![(
                    tab_id,
                    "Tab 1".to_string(),
                    vec![(pane_id, Some(root))],
                    pane_id,
                    Vec::<PaneId>::new(),
                    (0u32, vec![(0u32, pane_id)]),
                )],
                tab_id,
                u64::MAX,
            )],
            Some(workspace_id),
        ))
        .unwrap()
    }

    fn wait_for_connection(endpoint: &Endpoint) {
        for _ in 0..100 {
            if endpoint.connect().is_ok() {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("server did not start");
    }

    #[test]
    fn invalid_snapshot_inputs_yield_an_empty_session() {
        let directory = std::env::temp_dir().join(format!(
            "murmur-server-invalid-snapshots-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        std::fs::create_dir_all(&directory).unwrap();

        let valid_empty = Session::new().snapshot().to_bytes().unwrap();
        let mut unsupported = valid_empty.clone();
        unsupported[..std::mem::size_of::<u32>()].copy_from_slice(&3u32.to_le_bytes());
        let structurally_invalid = structurally_invalid_snapshot(directory.clone());
        let transport_oversized = {
            let mut session = Session::new();
            let workspace_id = session
                .create_workspace(directory.clone())
                .expect("Workspace capacity");
            assert!(
                session.rename_workspace(workspace_id, "x".repeat(MAX_PERSISTED_SNAPSHOT_BYTES))
            );
            let bytes = session.snapshot().to_bytes().unwrap();
            assert!(bytes.len() > MAX_PERSISTED_SNAPSHOT_BYTES);
            bytes
        };
        assert!(
            Session::restore(
                murmur_core::SessionSnapshot::from_bytes(&structurally_invalid).unwrap()
            )
            .is_err()
        );
        let cases = [
            ("missing", None),
            ("empty", Some(Vec::new())),
            ("corrupt", Some(vec![0xff, 0x00, 0x7f])),
            (
                "oversized",
                Some(vec![
                    0;
                    (crate::persistence::MAX_SNAPSHOT_BYTES + 1) as usize
                ]),
            ),
            ("unsupported", Some(unsupported)),
            ("structurally-invalid", Some(structurally_invalid)),
            ("transport-oversized", Some(transport_oversized)),
            ("valid-empty", Some(valid_empty)),
        ];

        for (name, bytes) in cases {
            let path = directory.join(format!("{name}.snapshot"));
            if let Some(bytes) = bytes {
                std::fs::write(&path, bytes).unwrap();
            }
            let (state, startup_terminals) =
                RuntimeState::recover(&test_endpoint(), Some(path)).unwrap();
            assert_eq!(
                state.session.snapshot(),
                Session::new().snapshot(),
                "{name}"
            );
            assert!(state.terminals.is_empty(), "{name}");
            assert!(startup_terminals.is_empty(), "{name}");
        }

        let _ = std::fs::remove_dir_all(directory);
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn restart_falls_back_from_a_missing_pane_cwd_and_persists_the_repair() {
        let directory = std::env::temp_dir().join(format!(
            "murmur-server-cwd-fallback-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let workspace_root = directory.join("workspace");
        let missing_cwd = workspace_root.join("missing");
        let snapshot_path = directory.join("session.snapshot");
        std::fs::create_dir_all(&workspace_root).unwrap();

        let mut session = Session::new();
        session
            .create_workspace(workspace_root.clone())
            .expect("Workspace capacity");
        let pane_id = session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id();
        assert!(session.set_pane_cwd(pane_id, Some(missing_cwd)));
        let absent_cwd_pane = session
            .split_pane(pane_id, murmur_core::SplitDirection::Horizontal, 0.5)
            .unwrap();
        assert!(session.set_pane_cwd(absent_cwd_pane, None));
        persist_snapshot(snapshot_path.clone(), session.snapshot());

        let endpoint = test_endpoint();
        let server = BoundServer::bind(
            ServerConfig::new(endpoint.clone()).with_snapshot_path(snapshot_path.clone()),
        )
        .unwrap();
        assert_eq!(server.startup_terminals.len(), 2);
        let recovered_before_runtime = Session::restore(server.handle().snapshot()).unwrap();
        assert_eq!(
            recovered_before_runtime
                .pane(absent_cwd_pane)
                .and_then(|pane| pane.cwd()),
            None
        );
        let handle = server.handle();
        let thread = thread::spawn(move || server.run());
        wait_for_connection(&endpoint);
        let connection = ClientConnection::connect(&endpoint, "cwd-fallback").unwrap();
        let repaired_session = Session::restore(connection.bootstrap().snapshot.clone()).unwrap();
        assert_eq!(
            repaired_session.pane(pane_id).and_then(|pane| pane.cwd()),
            Some(workspace_root.as_path())
        );
        assert_eq!(connection.bootstrap().terminals.len(), 2);
        let repaired_snapshot = connection.bootstrap().snapshot.clone();

        drop(connection);
        handle.stop();
        thread.join().unwrap().unwrap();
        let persisted =
            murmur_core::SessionSnapshot::from_bytes(&std::fs::read(&snapshot_path).unwrap())
                .unwrap();
        assert_eq!(persisted, repaired_snapshot);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn restart_prunes_only_failed_panes_and_persists_the_repair() {
        use murmur_core::SplitDirection;

        let directory = std::env::temp_dir().join(format!(
            "murmur-server-partial-restore-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let workspace_root = directory.join("missing-root");
        let valid_cwd = directory.join("valid");
        let missing_cwd = directory.join("missing-pane");
        let snapshot_path = directory.join("session.snapshot");
        std::fs::create_dir_all(&valid_cwd).unwrap();

        let mut session = Session::new();
        let workspace_id = session
            .create_workspace(workspace_root)
            .expect("Workspace capacity");
        assert!(session.rename_workspace(workspace_id, "Recovered Workspace"));
        let tab_id = session.active_workspace().unwrap().active_tab().id();
        assert!(session.rename_tab(tab_id, "Recovered Tab"));
        let surviving_pane = session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id();
        assert!(session.set_pane_cwd(surviving_pane, Some(valid_cwd.clone())));
        let failed_pane = session
            .split_pane(surviving_pane, SplitDirection::Vertical, 0.65)
            .unwrap();
        assert!(session.set_pane_cwd(failed_pane, Some(missing_cwd)));
        let persisted = session.snapshot();
        persist_snapshot(snapshot_path.clone(), persisted);

        let endpoint = test_endpoint();
        let server = BoundServer::bind(
            ServerConfig::new(endpoint.clone()).with_snapshot_path(snapshot_path.clone()),
        )
        .unwrap();
        assert_eq!(server.startup_terminals.len(), 1);
        let handle = server.handle();
        let thread = thread::spawn(move || server.run());
        wait_for_connection(&endpoint);
        let connection = ClientConnection::connect(&endpoint, "partial-restore").unwrap();
        let bootstrap = connection.bootstrap();
        assert_eq!(bootstrap.terminals.len(), 1);
        assert_eq!(bootstrap.terminals[0].pane_id, surviving_pane);
        assert!(bootstrap.agents.is_empty());
        let restored = Session::restore(bootstrap.snapshot.clone()).unwrap();
        assert_eq!(restored.active_workspace_id(), Some(workspace_id));
        assert_eq!(restored.workspaces().len(), 1);
        let workspace = restored.active_workspace().unwrap();
        assert_eq!(workspace.name(), "Recovered Workspace");
        assert_eq!(workspace.tabs().len(), 1);
        assert_eq!(workspace.active_tab().id(), tab_id);
        assert_eq!(workspace.active_tab().name(), "Recovered Tab");
        assert_eq!(workspace.active_tab().panes().len(), 1);
        assert_eq!(workspace.active_tab().focused_pane().id(), surviving_pane);
        assert_eq!(
            workspace.active_tab().layout(),
            &murmur_core::PaneLayout::Pane(surviving_pane)
        );
        assert_eq!(
            workspace.active_tab().focused_pane().cwd(),
            Some(valid_cwd.as_path())
        );
        let repaired_snapshot = bootstrap.snapshot.clone();

        drop(connection);
        handle.stop();
        thread.join().unwrap().unwrap();
        let repaired =
            murmur_core::SessionSnapshot::from_bytes(&std::fs::read(&snapshot_path).unwrap())
                .unwrap();
        assert_eq!(repaired, repaired_snapshot);
        let _ = std::fs::remove_dir_all(directory);
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn wholly_unrestorable_snapshot_persists_start_page_state() {
        let directory = std::env::temp_dir().join(format!(
            "murmur-server-empty-restore-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let snapshot_path = directory.join("session.snapshot");
        std::fs::create_dir_all(&directory).unwrap();
        let mut session = Session::new();
        session
            .create_workspace(directory.join("missing"))
            .expect("Workspace capacity");
        persist_snapshot(snapshot_path.clone(), session.snapshot());

        let endpoint = test_endpoint();
        let server = BoundServer::bind(
            ServerConfig::new(endpoint.clone()).with_snapshot_path(snapshot_path.clone()),
        )
        .unwrap();
        assert!(server.startup_terminals.is_empty());
        let handle = server.handle();
        let thread = thread::spawn(move || server.run());
        wait_for_connection(&endpoint);
        let connection = ClientConnection::connect(&endpoint, "empty-restore").unwrap();
        assert_eq!(connection.bootstrap().snapshot, Session::new().snapshot());
        assert!(connection.bootstrap().terminals.is_empty());

        drop(connection);
        handle.stop();
        thread.join().unwrap().unwrap();
        let repaired =
            murmur_core::SessionSnapshot::from_bytes(&std::fs::read(&snapshot_path).unwrap())
                .unwrap();
        assert_eq!(repaired, Session::new().snapshot());
        let _ = std::fs::remove_dir_all(directory);
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn restart_revalidates_worktree_authority_against_the_git_topology() {
        let directory = std::env::temp_dir().join(format!(
            "murmur-server-worktree-restore-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let repository = directory.join("repository");
        let parent_workspace_root = repository.join("workspace-root");
        let child_root = directory.join("child");
        let snapshot_path = directory.join("session.snapshot");
        std::fs::create_dir_all(&repository).unwrap();
        run_git(&repository, &["init"]);
        run_git(&repository, &["config", "user.name", "Murmur Tests"]);
        run_git(
            &repository,
            &["config", "user.email", "murmur@example.invalid"],
        );
        std::fs::create_dir_all(&parent_workspace_root).unwrap();
        std::fs::write(parent_workspace_root.join("README.md"), "murmur\n").unwrap();
        run_git(&repository, &["add", "workspace-root/README.md"]);
        run_git(&repository, &["commit", "-m", "initial"]);
        run_git(
            &repository,
            &[
                "worktree",
                "add",
                "-b",
                "feature/recovered",
                child_root.to_str().unwrap(),
            ],
        );

        let mut session = Session::new();
        let parent_workspace_id = session
            .create_workspace(parent_workspace_root.clone())
            .expect("Workspace capacity");
        let child_workspace_id = session
            .create_workspace(child_root.clone())
            .expect("Workspace capacity");
        assert!(session.associate_worktree(
            child_workspace_id,
            parent_workspace_id,
            parent_workspace_root,
            true,
        ));
        session
            .close_workspace(parent_workspace_id)
            .expect("historical parent Workspace exists");
        persist_snapshot(snapshot_path.clone(), session.snapshot());

        let first_endpoint = test_endpoint();
        let first_server = BoundServer::bind(
            ServerConfig::new(first_endpoint.clone()).with_snapshot_path(snapshot_path.clone()),
        )
        .unwrap();
        let first_handle = first_server.handle();
        let first_thread = thread::spawn(move || first_server.run());
        wait_for_connection(&first_endpoint);
        let first = ClientConnection::connect(&first_endpoint, "valid-worktree-restore").unwrap();
        let first_session = Session::restore(first.bootstrap().snapshot.clone()).unwrap();
        assert!(
            first_session
                .workspace(child_workspace_id)
                .and_then(|workspace| workspace.worktree())
                .is_some_and(|association| association.is_managed())
        );
        drop(first);
        first_handle.stop();
        first_thread.join().unwrap().unwrap();

        run_git(
            &repository,
            &["worktree", "remove", child_root.to_str().unwrap()],
        );
        std::fs::create_dir_all(&child_root).unwrap();

        let second_endpoint = test_endpoint();
        let second_server = BoundServer::bind(
            ServerConfig::new(second_endpoint.clone()).with_snapshot_path(snapshot_path.clone()),
        )
        .unwrap();
        let second_handle = second_server.handle();
        let second_thread = thread::spawn(move || second_server.run());
        wait_for_connection(&second_endpoint);
        let second = ClientConnection::connect(&second_endpoint, "stale-worktree-restore").unwrap();
        let repaired = Session::restore(second.bootstrap().snapshot.clone()).unwrap();
        assert!(
            repaired
                .workspace(child_workspace_id)
                .expect("child Workspace survives")
                .worktree()
                .is_none()
        );
        drop(second);
        second_handle.stop();
        second_thread.join().unwrap().unwrap();

        let persisted = murmur_core::SessionSnapshot::from_bytes(
            &std::fs::read(&snapshot_path).expect("repaired Snapshot exists"),
        )
        .unwrap();
        assert!(
            Session::restore(persisted)
                .unwrap()
                .workspace(child_workspace_id)
                .expect("child Workspace remains persisted")
                .worktree()
                .is_none()
        );
        let _ = std::fs::remove_dir_all(directory);
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn server_restart_restores_structure_with_fresh_terminal_state() {
        use murmur_core::SplitDirection;

        let directory = std::env::temp_dir().join(format!(
            "murmur-server-restart-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let workspace_root = directory.join("workspace");
        let workspace_cwd = workspace_root.join("nested");
        let snapshot_path = directory.join("session.snapshot");
        std::fs::create_dir_all(&workspace_cwd).unwrap();
        run_git(&workspace_root, &["init"]);
        run_git(&workspace_root, &["config", "user.name", "Murmur Tests"]);
        run_git(
            &workspace_root,
            &["config", "user.email", "murmur@example.invalid"],
        );
        std::fs::write(workspace_root.join("README.md"), "murmur\n").unwrap();
        run_git(&workspace_root, &["add", "README.md"]);
        run_git(&workspace_root, &["commit", "-m", "initial"]);
        run_git(&workspace_root, &["checkout", "-b", "ph6-restore"]);
        let endpoint = test_endpoint();

        let server = BoundServer::bind(
            ServerConfig::new(endpoint.clone()).with_snapshot_path(snapshot_path.clone()),
        )
        .unwrap();
        let first_handle = server.handle();
        let first_thread = thread::spawn(move || server.run());
        wait_for_connection(&endpoint);
        let first = ClientConnection::connect(&endpoint, "restart-first").unwrap();
        let initial = first.bootstrap().clone();
        let first_server_id = initial.server_id;
        let first_epoch = initial.runtime_epoch;
        let session_id = initial.session_id;
        let mut stream = first.into_stream();
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
                after_sequence: initial.sequence,
            },
        )
        .unwrap();
        assert!(matches!(
            read_server(&mut stream),
            ServerMessage::Subscribed { .. }
        ));

        let (restored_workspace_id, first_pane) = {
            let mut mutate = |command| {
                murmur_core::protocol::write_message(
                    &mut stream,
                    &ClientMessage::Layout {
                        server_id: first_server_id,
                        session_id,
                        command,
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
            };
            mutate(LayoutCommand::CreateWorkspace {
                root_directory: workspace_root.clone(),
            });
            let (workspace_id, tab_id, first_pane) = {
                let state = first_handle.state.lock().unwrap();
                let workspace = state.session.active_workspace().unwrap();
                (
                    workspace.id(),
                    workspace.active_tab().id(),
                    workspace.active_tab().focused_pane().id(),
                )
            };
            mutate(LayoutCommand::RenameWorkspace {
                workspace_id,
                name: "Persistent Workspace".into(),
            });
            mutate(LayoutCommand::RenameTab {
                tab_id,
                name: "Persistent Tab".into(),
            });
            mutate(LayoutCommand::SplitPane {
                pane_id: first_pane,
                direction: SplitDirection::Horizontal,
            });
            mutate(LayoutCommand::SetSplitRatios {
                tab_id,
                ratios: vec![0.7],
            });
            mutate(LayoutCommand::FocusPane {
                pane_id: first_pane,
            });
            (workspace_id, first_pane)
        };
        let sequence_after_layout = first_handle.state.lock().unwrap().sequence;
        let cwd_command = if cfg!(windows) {
            format!(
                "Set-Location -LiteralPath '{}'; Write-Output ('MURMUR_' + 'CWD_CHANGED')\r",
                workspace_cwd.to_string_lossy().replace('\'', "''")
            )
        } else {
            format!(
                "cd '{}' && printf 'MURMUR_%s\\n' CWD_CHANGED\r",
                workspace_cwd.to_string_lossy().replace('\'', "'\\''")
            )
        };
        send_terminal(
            &mut stream,
            first_server_id,
            session_id,
            first_pane,
            TerminalCommand::Text(cwd_command),
        );
        let deadline = Instant::now() + Duration::from_secs(8);
        let expected = loop {
            let state = first_handle.state.lock().unwrap();
            if state.session.pane(first_pane).and_then(|pane| pane.cwd())
                == Some(workspace_cwd.as_path())
            {
                assert_eq!(state.sequence, sequence_after_layout);
                break state.session.snapshot();
            }
            drop(state);
            assert!(
                Instant::now() < deadline,
                "authoritative Session cwd never became {}",
                workspace_cwd.display()
            );
            thread::sleep(Duration::from_millis(20));
        };

        send_terminal(
            &mut stream,
            first_server_id,
            session_id,
            first_pane,
            TerminalCommand::Text("echo MURMUR_OLD_RUNTIME_MARKER\r".into()),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let contains_marker = {
                let state = first_handle.state.lock().unwrap();
                state.terminals.get(&first_pane).is_some_and(|runtime| {
                    view_text(&runtime.view()).contains("MURMUR_OLD_RUNTIME_MARKER")
                })
            };
            if contains_marker {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "old terminal runtime never displayed its marker"
            );
            thread::sleep(Duration::from_millis(20));
        }

        first_handle.stop();
        drop(stream);
        first_thread.join().unwrap().unwrap();
        assert_eq!(
            murmur_core::SessionSnapshot::from_bytes(&std::fs::read(&snapshot_path).unwrap())
                .unwrap(),
            expected
        );

        let replacement = BoundServer::bind(
            ServerConfig::new(endpoint.clone()).with_snapshot_path(snapshot_path),
        )
        .unwrap();
        let replacement_handle = replacement.handle();
        let replacement_thread = thread::spawn(move || replacement.run());
        wait_for_connection(&endpoint);
        let restored = ClientConnection::connect(&endpoint, "restart-second").unwrap();
        let bootstrap = restored.bootstrap().clone();
        assert_eq!(bootstrap.server_id, first_server_id);
        assert_ne!(bootstrap.runtime_epoch, first_epoch);
        assert_eq!(bootstrap.sequence, 0);
        assert_eq!(bootstrap.snapshot, expected);
        assert_eq!(bootstrap.terminals.len(), 2);
        assert!(bootstrap.agents.is_empty());
        assert!(bootstrap.workspace_git.iter().any(|git| {
            git.workspace_id == restored_workspace_id
                && git.branch.as_deref() == Some("ph6-restore")
                && !git.linked_worktree
        }));
        assert!(
            bootstrap.terminals.iter().all(|terminal| {
                !view_text(&terminal.view).contains("MURMUR_OLD_RUNTIME_MARKER")
            })
        );
        let restored_session = Session::restore(bootstrap.snapshot.clone()).unwrap();
        assert_eq!(
            restored_session
                .pane(first_pane)
                .and_then(|pane| pane.cwd()),
            Some(workspace_cwd.as_path())
        );

        let mut restored_stream = restored.into_stream();
        restored_stream
            .set_handshake_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        murmur_core::protocol::write_message(
            &mut restored_stream,
            &ClientMessage::AcquireControl { session_id },
        )
        .unwrap();
        assert!(matches!(
            read_server(&mut restored_stream),
            ServerMessage::ControlGranted { .. }
        ));
        let cwd_check = if cfg!(windows) {
            format!(
                "if ((Get-Location).Path -eq '{}') {{ Write-Output ('MURMUR_RESTORED_' + 'CWD_OK') }} else {{ Write-Output ('MURMUR_RESTORED_' + 'CWD_BAD') }}\r",
                workspace_cwd.to_string_lossy().replace('\'', "''")
            )
        } else {
            format!(
                "if [ \"$PWD\" = '{}' ]; then printf 'MURMUR_RESTORED_%s\\n' CWD_OK; else printf 'MURMUR_RESTORED_%s\\n' CWD_BAD; fi\r",
                workspace_cwd.to_string_lossy().replace('\'', "'\\''")
            )
        };
        send_terminal(
            &mut restored_stream,
            first_server_id,
            session_id,
            first_pane,
            TerminalCommand::Text(cwd_check),
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        let restored_text = loop {
            let text = {
                let state = replacement_handle.state.lock().unwrap();
                state
                    .terminals
                    .get(&first_pane)
                    .map(|runtime| view_text(&runtime.view()))
                    .unwrap()
            };
            if text.contains("MURMUR_RESTORED_CWD_") {
                break text;
            }
            assert!(
                Instant::now() < deadline,
                "fresh shell did not report its working directory"
            );
            thread::sleep(Duration::from_millis(20));
        };
        assert!(
            restored_text.contains("MURMUR_RESTORED_CWD_OK"),
            "fresh shell did not start in {}",
            workspace_cwd.display()
        );

        replacement_handle.stop();
        drop(restored_stream);
        replacement_thread.join().unwrap().unwrap();
        let _ = std::fs::remove_dir_all(directory);
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    #[test]
    fn terminal_tail_cwd_survives_exit_and_shutdown() {
        #[cfg(target_os = "linux")]
        if std::path::Path::new(&murmur_core::CommandBuilder::new_default_prog().get_shell())
            .file_name()
            .and_then(|name| name.to_str())
            != Some("bash")
        {
            return;
        }

        let directory = std::env::temp_dir().join(format!(
            "murmur-server-exit-cwd-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        let workspace_root = directory.join("workspace");
        let final_cwd = workspace_root.join("final");
        let snapshot_path = directory.join("session.snapshot");
        std::fs::create_dir_all(&final_cwd).unwrap();

        let mut session = Session::new();
        session
            .create_workspace(workspace_root)
            .expect("Workspace capacity");
        let pane_id = session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id();
        persist_snapshot(snapshot_path.clone(), session.snapshot());

        let endpoint = test_endpoint();
        let server = BoundServer::bind(
            ServerConfig::new(endpoint.clone()).with_snapshot_path(snapshot_path.clone()),
        )
        .unwrap();
        let handle = server.handle();
        let thread = thread::spawn(move || server.run());
        wait_for_connection(&endpoint);
        let connection = ClientConnection::connect(&endpoint, "exit-cwd").unwrap();
        let server_id = connection.bootstrap().server_id;
        let session_id = connection.bootstrap().session_id;
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

        #[cfg(target_os = "linux")]
        {
            let change_cwd_and_exit = format!(
                "cd '{}'; exit",
                final_cwd.to_string_lossy().replace('\'', "'\\''")
            );
            send_terminal(
                &mut stream,
                server_id,
                session_id,
                pane_id,
                TerminalCommand::Text(change_cwd_and_exit),
            );
            send_terminal(
                &mut stream,
                server_id,
                session_id,
                pane_id,
                TerminalCommand::Key {
                    key: murmur_core::TerminalKey::Enter,
                    modifiers: murmur_core::TerminalModifiers::default(),
                },
            );
        }
        #[cfg(target_os = "windows")]
        {
            let change_cwd = format!(
                "Set-Location -LiteralPath '{}'",
                final_cwd.to_string_lossy().replace('\'', "''")
            );
            send_terminal(
                &mut stream,
                server_id,
                session_id,
                pane_id,
                TerminalCommand::Text(change_cwd),
            );
            send_terminal(
                &mut stream,
                server_id,
                session_id,
                pane_id,
                TerminalCommand::Key {
                    key: murmur_core::TerminalKey::Enter,
                    modifiers: murmur_core::TerminalModifiers::default(),
                },
            );

            let probe_deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let state = handle.state.lock().unwrap();
                let probed = state.terminals.get(&pane_id).and_then(TerminalRuntime::cwd);
                if probed.as_deref() == Some(final_cwd.as_path()) {
                    assert_ne!(
                        state.session.pane(pane_id).and_then(|pane| pane.cwd()),
                        Some(final_cwd.as_path()),
                        "the low-frequency scan ran before the shutdown-path assertion"
                    );
                    break;
                }
                drop(state);
                assert!(
                    Instant::now() < probe_deadline,
                    "terminal cwd probe never observed {}",
                    final_cwd.display()
                );
                thread::sleep(Duration::from_millis(5));
            }
            send_terminal(
                &mut stream,
                server_id,
                session_id,
                pane_id,
                TerminalCommand::Text("exit".into()),
            );
            send_terminal(
                &mut stream,
                server_id,
                session_id,
                pane_id,
                TerminalCommand::Key {
                    key: murmur_core::TerminalKey::Enter,
                    modifiers: murmur_core::TerminalModifiers::default(),
                },
            );
        }

        #[cfg(target_os = "linux")]
        {
            let deadline = Instant::now() + Duration::from_secs(8);
            loop {
                let state = handle.state.lock().unwrap();
                if state.exited_terminals.contains(&pane_id) {
                    assert_eq!(
                        state.session.pane(pane_id).and_then(|pane| pane.cwd()),
                        Some(final_cwd.as_path())
                    );
                    break;
                }
                if Instant::now() >= deadline {
                    let terminal = state
                        .terminals
                        .get(&pane_id)
                        .map(TerminalRuntime::visible_text)
                        .unwrap_or_default();
                    panic!("terminal never reported exit: {terminal:?}");
                }
                drop(state);
                thread::sleep(Duration::from_millis(20));
            }
        }
        #[cfg(target_os = "windows")]
        {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let state = handle.state.lock().unwrap();
                let terminal = state
                    .terminals
                    .get(&pane_id)
                    .map(TerminalRuntime::visible_text)
                    .unwrap_or_default();
                if terminal.contains("> exit") {
                    break;
                }
                drop(state);
                assert!(
                    Instant::now() < deadline,
                    "terminal never received exit: {terminal:?}"
                );
                thread::sleep(Duration::from_millis(20));
            }
        }

        handle.stop();
        drop(stream);
        thread.join().unwrap().unwrap();
        let persisted =
            murmur_core::SessionSnapshot::from_bytes(&std::fs::read(snapshot_path).unwrap())
                .unwrap();
        assert_eq!(
            Session::restore(persisted)
                .unwrap()
                .pane(pane_id)
                .and_then(|pane| pane.cwd()),
            Some(final_cwd.as_path())
        );
        let _ = std::fs::remove_dir_all(directory);
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
        let ServerMessage::Bootstrap(header) = bootstrap else {
            panic!("expected Bootstrap header");
        };
        let batch_count = header.batch_count;
        let mut assembler = BootstrapAssembler::new(header).unwrap();
        for _ in 0..batch_count {
            let message: ServerMessage = murmur_core::protocol::read_message(&mut stream).unwrap();
            let ServerMessage::BootstrapBatch(batch) = message else {
                panic!("expected Bootstrap batch");
            };
            assembler.push(batch).unwrap();
        }
        assembler.finish().unwrap();
        stream
    }

    #[cfg(any(target_os = "linux", target_os = "windows"))]
    fn apply_for_test(
        state: &mut RuntimeState,
        updates: &mut Vec<mpsc::Receiver<TerminalUpdate>>,
        command: LayoutCommand,
    ) -> usize {
        let terminal_count_before = state.terminals.len();
        let plan = external_layout_plan(state, &command).unwrap();
        let effect = match plan {
            Some(plan) => {
                let mut prepared = prepare_external_layout(plan).unwrap();
                approve_external_layout(state, &mut prepared).unwrap();
                let prepared = finish_external_layout(prepared)
                    .unwrap_or_else(|failure| panic!("{}", failure.message));
                apply_prepared_external_layout(state, prepared).unwrap()
            }
            None => apply_layout_command(state, command).unwrap(),
        };
        for (_, _, receiver, _, _, _) in effect.started_terminals {
            updates.push(receiver);
        }
        let removed_count = terminal_count_before.saturating_sub(state.terminals.len());
        drop(effect.removed_terminals);
        removed_count
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
    fn failed_managed_worktree_removal_restarts_its_live_terminals() {
        let temp = std::env::temp_dir().join(format!(
            "murmur-server-worktree-recovery-{}-{}",
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
                branch: "feature/removal-recovery".into(),
            },
        );
        let child_workspace_id = state.session.active_workspace_id().unwrap();
        let child = state.session.workspace(child_workspace_id).unwrap();
        let child_root = child.root_directory().to_path_buf();
        let pane_id = child.active_tab().focused_pane().id();
        let previous_instance = state.terminal_instances[&pane_id];

        let command = LayoutCommand::RemoveWorktree {
            workspace_id: child_workspace_id,
        };
        let plan = external_layout_plan(&state, &command).unwrap().unwrap();
        let mut prepared = prepare_external_layout(plan).unwrap();
        std::fs::write(child_root.join("became-dirty.txt"), "dirty\n").unwrap();
        approve_external_layout(&mut state, &mut prepared).unwrap();
        assert!(!state.terminals.contains_key(&pane_id));
        assert!(!state.terminal_instances.contains_key(&pane_id));

        let failure = match finish_external_layout(prepared) {
            Ok(_) => panic!("dirty worktree removal unexpectedly succeeded"),
            Err(failure) => failure,
        };
        assert!(failure.message.contains("modified or untracked files"));
        assert_eq!(failure.restarted.len(), 1);
        for (pane_id, runtime, receiver) in failure.restarted {
            let started = state.install_terminal(pane_id, runtime, receiver);
            updates.push(started.2);
        }
        assert!(state.terminals.contains_key(&pane_id));
        assert_ne!(state.terminal_instances[&pane_id], previous_instance);
        assert!(state.session.workspace(child_workspace_id).is_some());

        std::fs::remove_file(child_root.join("became-dirty.txt")).unwrap();
        apply_for_test(
            &mut state,
            &mut updates,
            LayoutCommand::RemoveWorktree {
                workspace_id: child_workspace_id,
            },
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
        let parent_workspace_id = state
            .session
            .create_workspace(repository.clone())
            .expect("Workspace capacity");
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

        cancel_prepared_external_layout(prepared).unwrap();
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
        let workspace_id = state
            .session
            .create_workspace(repository.clone())
            .expect("Workspace capacity");
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
        let parent_workspace_id = state
            .session
            .create_workspace(repository.clone())
            .expect("Workspace capacity");
        let child_workspace_id = state
            .session
            .create_workspace(worktree.clone())
            .expect("Workspace capacity");
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
    fn rejected_subscription_is_typed_and_removes_the_previous_subscriber() {
        let (handle, endpoint, thread) = start();
        let mut stream = connect_and_bootstrap(&endpoint);
        let (server_id, session_id) = {
            let state = handle.state.lock().unwrap();
            (state.server_id, state.session_id)
        };

        murmur_core::protocol::write_message(
            &mut stream,
            &ClientMessage::Subscribe {
                session_id,
                after_sequence: 0,
            },
        )
        .unwrap();
        assert!(matches!(
            murmur_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap(),
            ServerMessage::Subscribed { .. }
        ));
        assert_eq!(handle.state.lock().unwrap().subscribers.len(), 1);

        murmur_core::protocol::write_message(
            &mut stream,
            &ClientMessage::Subscribe {
                session_id,
                after_sequence: u64::MAX,
            },
        )
        .unwrap();
        assert!(matches!(
            murmur_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap(),
            ServerMessage::SubscriptionRejected {
                server_id: rejected_server,
                session_id: rejected_session,
                reason,
            } if rejected_server == server_id
                && rejected_session == session_id
                && reason == "event cursor is ahead of the server"
        ));
        assert!(handle.state.lock().unwrap().subscribers.is_empty());

        {
            let mut state = handle.state.lock().unwrap();
            for _ in 0..=EVENT_HISTORY_LIMIT {
                state.publish_background(SessionEvent::LayoutChanged);
            }
        }
        murmur_core::protocol::write_message(
            &mut stream,
            &ClientMessage::Subscribe {
                session_id: SessionId(session_id.0.wrapping_add(1)),
                after_sequence: 0,
            },
        )
        .unwrap();
        assert!(matches!(
            murmur_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap(),
            ServerMessage::SubscriptionRejected {
                server_id: rejected_server,
                session_id: authoritative_session,
                reason,
            } if rejected_server == server_id
                && authoritative_session == session_id
                && reason == "unknown Session"
        ));

        murmur_core::protocol::write_message(
            &mut stream,
            &ClientMessage::Subscribe {
                session_id,
                after_sequence: 0,
            },
        )
        .unwrap();
        assert!(matches!(
            murmur_core::protocol::read_message::<_, ServerMessage>(&mut stream).unwrap(),
            ServerMessage::SubscriptionRejected {
                server_id: rejected_server,
                session_id: rejected_session,
                reason,
            } if rejected_server == server_id
                && rejected_session == session_id
                && reason == "event cursor expired"
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

    #[cfg(target_os = "linux")]
    #[test]
    fn stop_server_cancels_resize_queued_behind_pty_backpressure() {
        let (handle, endpoint, server_thread) = start();
        let connection = ClientConnection::connect(&endpoint, "blocked-resize").unwrap();
        let bootstrap = connection.bootstrap().clone();
        let server_id = bootstrap.server_id;
        let session_id = bootstrap.session_id;
        let mut controller = connection.into_stream();
        controller
            .set_handshake_timeout(Some(Duration::from_secs(5)))
            .unwrap();

        murmur_core::protocol::write_message(
            &mut controller,
            &ClientMessage::AcquireControl { session_id },
        )
        .unwrap();
        assert!(matches!(
            read_server(&mut controller),
            ServerMessage::ControlGranted { .. }
        ));
        murmur_core::protocol::write_message(
            &mut controller,
            &ClientMessage::Subscribe {
                session_id,
                after_sequence: bootstrap.sequence,
            },
        )
        .unwrap();
        assert!(matches!(
            read_server(&mut controller),
            ServerMessage::Subscribed { .. }
        ));
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
        wait_for_message(&mut controller, |message| {
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

        let stopper_connection =
            ClientConnection::connect(&endpoint, "blocked-resize-stop").unwrap();
        let mut stopper = stopper_connection.into_stream();
        stopper
            .set_handshake_timeout(Some(Duration::from_secs(2)))
            .unwrap();

        send_terminal(
            &mut controller,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Text(
                "stty raw -echo; printf 'murmur-writer-blocked\\r\\n'; sleep 30\r".into(),
            ),
        );
        let mut views = std::collections::HashMap::new();
        wait_for_terminal_text(
            &mut controller,
            &mut views,
            pane_id,
            "murmur-writer-blocked",
        );
        send_terminal(
            &mut controller,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Text("x".repeat(1024 * 1024)),
        );
        send_terminal(
            &mut controller,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Resize(TerminalSize::new(10, 40)),
        );
        thread::sleep(Duration::from_millis(100));

        murmur_core::protocol::write_message(
            &mut stopper,
            &ClientMessage::StopServer { server_id },
        )
        .unwrap();
        assert_eq!(read_server(&mut stopper), ServerMessage::ServerStopping);
        drop(controller);
        drop(stopper);

        let deadline = Instant::now() + Duration::from_secs(2);
        while !server_thread.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            server_thread.is_finished(),
            "Server stop waited for a blocked Terminal resize"
        );
        server_thread.join().unwrap().unwrap();
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

        let checkout_deadline = Instant::now() + Duration::from_secs(10);
        let checkout_started = loop {
            if marker.exists() {
                break true;
            }
            if Instant::now() >= checkout_deadline {
                break false;
            }
            thread::sleep(Duration::from_millis(10));
        };
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
        let server = BoundServer::bind(ServerConfig::ephemeral(Endpoint::tcp(
            "127.0.0.1:0".parse().unwrap(),
        )))
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

        let server = BoundServer::bind(ServerConfig::ephemeral(Endpoint::tcp(
            "127.0.0.1:0".parse().unwrap(),
        )))
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
