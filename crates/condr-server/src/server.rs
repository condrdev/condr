use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use condr_core::protocol::{
    BootstrapAssembler, BootstrapBatch, BootstrapHeader, BootstrapRecord, ClientMessage,
    FramingError, Hello, LayoutCommand, MAX_BOOTSTRAP_BATCHES, MAX_BOOTSTRAP_TOTAL_SIZE,
    MAX_CHUNK_PAYLOAD_SIZE, MAX_FRAME_SIZE, PROTOCOL_VERSION, PaneAgentSnapshot, PaneTerminalFrame,
    PaneTerminalSnapshot, RuntimeEpoch, ServerId, ServerMessage, SessionBootstrap, SessionEvent,
    SessionId, TerminalFrameBatch, TerminalFrameChunk, VersionCheck, WorkspaceGitSnapshot,
    check_version, encode_bootstrap_record, encode_pane_terminal_frame,
};
use condr_core::{
    AgentSnapshot, GitRepository, PaneId, Session, TerminalAgentProbe, TerminalCommand,
    TerminalCwdProbe, TerminalRuntime, TerminalSize, TerminalUpdate, TerminalView,
    TerminalViewFrame, TerminalViewSource, WorkspaceId, create_worktree, default_worktree_root,
    discover_repository, open_worktree, remove_worktree, validate_worktree_removal,
};

use crate::client_writer::{ClientWriteItem, ClientWriter};
use crate::endpoint::{Endpoint, EndpointListener, EndpointStream, default_socket_path};
use crate::persistence::{SnapshotLoad, SnapshotPersistence};

mod bootstrap;
mod client;
mod layout;
mod local;
mod terminal_monitor;

use bootstrap::*;
use client::*;
use layout::*;
#[cfg(test)]
use local::snapshot_path_for_endpoint;
use local::{default_snapshot_path, runtime_epoch, stable_endpoint_id};
pub use local::{ensure_local_server, ensure_server, probe_server, stop_server};
use terminal_monitor::*;

const ACCEPT_POLL: Duration = Duration::from_millis(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(4);
const STOP_ACK_TIMEOUT: Duration = Duration::from_secs(1);
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
        condr_core::protocol::write_message(
            &mut stream,
            &ClientMessage::Hello(Hello {
                version: PROTOCOL_VERSION,
                client_name: client_name.into(),
            }),
        )
        .map_err(|error| io::Error::other(error.to_string()))?;
        let welcome: ServerMessage = condr_core::protocol::read_message(&mut stream)
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
        let bootstrap = match condr_core::protocol::read_message(&mut stream)
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
            let batch = match condr_core::protocol::read_message(&mut stream)
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

    pub fn snapshot(&self) -> condr_core::SessionSnapshot {
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
}

pub struct BoundServer {
    listener: EndpointListener,
    stop: Arc<AtomicBool>,
    lifecycle: Arc<ServerLifecycle>,
    state: Arc<Mutex<RuntimeState>>,
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
                    let _ = listener.cleanup();
                    return Err(error);
                }
            };
        Ok(Self {
            listener,
            stop: Arc::new(AtomicBool::new(false)),
            lifecycle: Arc::new(ServerLifecycle::default()),
            state: Arc::new(Mutex::new(state)),
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
        let mut next_client_id = 1_u64;
        while !self.stop.load(Ordering::Acquire) {
            match self.listener.accept() {
                Ok(stream) => {
                    let client_id = next_client_id;
                    next_client_id = next_client_id.wrapping_add(1);
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
                    "condr-server: failed to close Terminal for Pane {}: {error}",
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
            eprintln!("condr-server: final Session Snapshot flush failed: {error}");
        }
        drop(persistence);
        drop(terminals);
        let cleanup_result = self.listener.cleanup();
        if let Err(error) = &cleanup_result {
            eprintln!("condr-server: endpoint cleanup failed: {error}");
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
    operations: usize,
}

impl ServerLifecycle {
    fn begin_operation(self: &Arc<Self>) -> Option<OperationGuard> {
        let mut state = self.state.lock().expect("server lifecycle lock poisoned");
        if state.stopping {
            return None;
        }
        state.operations += 1;
        Some(OperationGuard {
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

    fn wait_for_operations(&self) {
        let state = self.state.lock().expect("server lifecycle lock poisoned");
        let _state = self
            .idle
            .wait_while(state, |state| state.operations != 0)
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
                "condr-server: clearing stale worktree association for Workspace {} at {}",
                workspace_id.as_u64(),
                root.display()
            );
            cleared += 1;
        }
    }
    cleared
}

struct OperationGuard {
    lifecycle: Arc<ServerLifecycle>,
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        let mut state = self
            .lifecycle
            .state
            .lock()
            .expect("server lifecycle lock poisoned");
        state.operations -= 1;
        if state.operations == 0 {
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
    snapshot: condr_core::SessionSnapshot,
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
            worktree_root: Some(default_worktree_root()),
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
                SnapshotLoad::Loaded(snapshot) => match validate_persistable_snapshot(&snapshot)
                    .and_then(|()| validate_snapshot_root_paths(&snapshot))
                {
                    Ok(()) => match Session::restore(snapshot) {
                        Ok(loaded) => {
                            session = loaded;
                            restored = true;
                        }
                        Err(error) => eprintln!(
                            "condr-server: ignoring invalid Session Snapshot at {}: {error}",
                            persistence.path().display()
                        ),
                    },
                    Err(error) => eprintln!(
                        "condr-server: ignoring invalid Session Snapshot at {}: {error}",
                        persistence.path().display()
                    ),
                },
                SnapshotLoad::Rejected(reason) => eprintln!(
                    "condr-server: ignoring invalid Session Snapshot at {}: {reason}",
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
            let requested_cwd = saved_cwd
                .as_deref()
                .filter(|cwd| cwd.is_absolute())
                .unwrap_or(workspace_root.as_path());
            let started = match TerminalRuntime::spawn_shell(
                requested_cwd,
                TerminalSize::new(24, 80),
            ) {
                Ok(runtime) => Some((runtime, requested_cwd.to_path_buf())),
                Err(error) if requested_cwd != workspace_root.as_path() => {
                    eprintln!(
                        "condr-server: fresh shell failed for Pane {} in saved cwd {}; retrying Workspace Root {}: {error}",
                        pane_id.as_u64(),
                        requested_cwd.display(),
                        workspace_root.display()
                    );
                    match TerminalRuntime::spawn_shell(&workspace_root, TerminalSize::new(24, 80)) {
                        Ok(runtime) => Some((runtime, workspace_root)),
                        Err(fallback_error) => {
                            eprintln!(
                                "condr-server: pruning Pane {} after fresh shell also failed in Workspace Root: {fallback_error}",
                                pane_id.as_u64()
                            );
                            None
                        }
                    }
                }
                Err(error) => {
                    eprintln!(
                        "condr-server: pruning Pane {} after fresh shell failed in {}: {error}",
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

    fn schedule_snapshot(&self, snapshot: condr_core::SessionSnapshot) {
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
                    "condr-server: ignoring unpersistable Terminal cwd update for Pane {}: path is not valid UTF-8",
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
                        "condr-server: ignoring unpersistable Terminal cwd update for Pane {}: {error}",
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

    fn publish_layout_change(
        &mut self,
        origin_client_id: u64,
        origin: &ClientWriter,
        request_id: u64,
    ) -> bool {
        let event = SessionEvent::LayoutChanged;
        if self.publish_event(event, Some((origin_client_id, origin))) {
            return true;
        }
        queue_message(
            origin,
            ServerMessage::LayoutApplied {
                server_id: self.server_id,
                session_id: self.session_id,
                request_id,
                sequence: self.sequence,
            },
        )
    }

    fn publish_background(&mut self, event: SessionEvent) {
        self.publish_event(event, None);
    }

    fn publish_event(&mut self, event: SessionEvent, origin: Option<(u64, &ClientWriter)>) -> bool {
        self.sequence = self.sequence.saturating_add(1);
        self.events.push_back(SequencedEvent {
            sequence: self.sequence,
            event: event.clone(),
        });
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
        match frame {
            TerminalViewFrame::Full(view) => {
                self.terminal_views.insert(
                    pane_id,
                    LatestTerminalView {
                        view: Arc::new(view),
                        producer_frame: None,
                    },
                );
            }
            TerminalViewFrame::Delta(delta) => {
                let latest = self.terminal_views.get_mut(&pane_id)?;
                Arc::make_mut(&mut latest.view)
                    .apply_frame(TerminalViewFrame::Delta(delta.clone()))
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

#[cfg(test)]
mod tests;
