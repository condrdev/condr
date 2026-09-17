mod agents;
mod bootstrap;
mod client;
mod clipboard_image;
mod config;
mod layout;
mod local;
mod recovery;
mod subscriptions;
mod terminal_monitor;
mod terminal_state;
mod terminal_stream;
mod workspace_git;

use crate::ClientConnection;
use crate::client_writer::{ClientWriteItem, ClientWriter, ReliableSendError};
use crate::endpoint::{
    Endpoint, EndpointListener, EndpointStream, TcpEndpoint, default_socket_path,
};
use crate::noise::{ServerIdentity, StaticKey};
use crate::persistence::{SnapshotLoad, SnapshotPersistence};
use agents::AgentControl;
use bootstrap::*;
use client::*;
#[cfg(test)]
use condr_core::protocol::BootstrapAssembler;
use condr_core::protocol::{
    BootstrapBatch, BootstrapHeader, BootstrapRecord, ClientMessage, DiffBase, FramingError, Hello,
    LayoutCommand, LayoutResult, MAX_BOOTSTRAP_BATCHES, MAX_BOOTSTRAP_TOTAL_SIZE,
    MAX_CHUNK_PAYLOAD_SIZE, MAX_FRAME_SIZE, PROTOCOL_VERSION, PaneAgentSnapshot, PaneTerminalFrame,
    PaneTerminalMetadata, PaneTerminalSnapshot, RuntimeEpoch, ServerAdminCommand,
    ServerAdminResponse, ServerId, ServerMessage, ServerSettings, SessionBootstrap, SessionEvent,
    SessionId, SessionOverview, TerminalFrameBatch, TerminalFrameChunk, VersionCheck,
    WorkspaceGitSnapshot, check_version, encode_bootstrap_record, encode_pane_terminal_frame,
};
use condr_core::{
    AgentSnapshot, GitFingerprint, GitRepository, PaneEnvironment, PaneId, Session,
    TerminalAgentProbe, TerminalCommand, TerminalCwdProbe, TerminalHyperlinkBudget,
    TerminalNoticeBatch, TerminalNoticeProbe, TerminalRuntime, TerminalSize, TerminalUpdate,
    TerminalView, TerminalViewFrame, TerminalViewSource, WorkspaceId, create_worktree,
    default_worktree_root, discover_repository, open_worktree, remove_worktree,
    validate_worktree_removal,
};
use config::{load_shell, save_shell};
use layout::*;
#[cfg(test)]
use local::snapshot_path_for_endpoint;
use local::{default_snapshot_path, runtime_epoch, stable_endpoint_id};
use recovery::{observe_terminal_cwds, shutdown_cwd};
use relative_path::RelativePathBuf;
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use terminal_monitor::*;
use terminal_stream::{LatestTerminalView, flush_terminal_render};
#[cfg(test)]
use terminal_stream::{
    TerminalRenderPaneSnapshot, TerminalRenderSnapshot, frame_terminal_batches,
    prepare_terminal_render,
};
use workspace_git::*;

pub use config::{ServerConfig, load_listen, save_listen};
pub use local::{
    connected_devices, ensure_local_server, ensure_server, ensure_server_from, probe_server,
    restart_server, restart_server_from, revoke_devices, server_status, stop_server,
    wait_for_shutdown,
};

const ACCEPT_POLL: Duration = Duration::from_millis(10);
pub(crate) const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(4);
const STOP_ACK_TIMEOUT: Duration = Duration::from_secs(1);
const EVENT_HISTORY_LIMIT: usize = 256;
const MAX_SHELL_SETTING_BYTES: usize = 4 * 1024;
/// Process cwd reads are rate-limited behind terminal output; agent detection has its
/// own cadence inside `TerminalAgentProbe`.
const CWD_SCAN_INTERVAL: Duration = Duration::from_millis(500);
/// A pending native resume checks whether the shell is idle at most this often: the
/// check scans the whole process table, and a starting shell wakes the probe per chunk.
const RESUME_RETRY_INTERVAL: Duration = Duration::from_millis(500);
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
    TerminalNoticeProbe,
    TerminalViewSource,
);
type TerminalBaselines = Vec<(PaneId, Arc<TerminalView>)>;

struct TerminalMonitor {
    pane_id: PaneId,
    instance_id: u64,
    updates: mpsc::Receiver<TerminalUpdate>,
    agent_probe: TerminalAgentProbe,
    cwd_probe: TerminalCwdProbe,
    notice_probe: TerminalNoticeProbe,
    view_source: TerminalViewSource,
    state: Arc<Mutex<RuntimeState>>,
    lifecycle: Arc<ServerLifecycle>,
}

#[derive(Clone)]
pub struct ServerHandle {
    stop: Arc<AtomicBool>,
    lifecycle: Arc<ServerLifecycle>,
    state: Arc<Mutex<RuntimeState>>,
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
    local: EndpointListener,
    tcp: Option<EndpointListener>,
    /// What a test client connects to: the TCP listener as the test device when there is
    /// one, else the local socket.
    endpoint: Endpoint,
    stop: Arc<AtomicBool>,
    lifecycle: Arc<ServerLifecycle>,
    state: Arc<Mutex<RuntimeState>>,
    startup_terminals: Vec<StartedTerminal>,
}

impl BoundServer {
    pub fn bind(config: ServerConfig) -> io::Result<Self> {
        let local = EndpointListener::local(&config.socket_path)?;
        local.set_nonblocking(true)?;
        let mut endpoint = Endpoint::local(&config.socket_path);
        let mut tcp = None;
        if let Some(address) = config.listen {
            let identity = match config.identity {
                Some(identity) => identity,
                None => Arc::new(ServerIdentity::load_or_create(
                    &crate::noise::identity_directory()?,
                )?),
            };
            let listener = match EndpointListener::tcp(address, Arc::clone(&identity)) {
                Ok(listener) => listener,
                Err(error) => {
                    let _ = local.cleanup();
                    return Err(io::Error::new(
                        error.kind(),
                        format!("failed to listen on tcp://{address}: {error}"),
                    ));
                }
            };
            listener.set_nonblocking(true)?;
            if let (Some(client_key), Ok(Some(address))) =
                (config.test_device, listener.local_addr())
            {
                endpoint =
                    Endpoint::tcp(TcpEndpoint::at(address, identity.public_key(), client_key));
            }
            tcp = Some(listener);
        }
        let (state, startup_terminals) = match RuntimeState::recover(
            &config.socket_path,
            config.snapshot_path,
            config.config_path,
        ) {
            Ok(restored) => restored,
            Err(error) => {
                let _ = local.cleanup();
                return Err(error);
            }
        };
        let state = Arc::new(Mutex::new(state));
        state
            .lock()
            .expect("server state lock poisoned")
            .start_git_watcher(Arc::downgrade(&state));
        Ok(Self {
            local,
            tcp,
            endpoint,
            stop: Arc::new(AtomicBool::new(false)),
            lifecycle: Arc::new(ServerLifecycle::default()),
            state,
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

    /// The TCP address actually bound, once `listen` asked for one.
    pub fn local_addr(&self) -> io::Result<Option<std::net::SocketAddr>> {
        self.tcp
            .as_ref()
            .map_or(Ok(None), EndpointListener::local_addr)
    }

    /// The endpoint a test client connects to: TCP as the authorized test device when
    /// `ephemeral_tcp` configured one, else the local socket.
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
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
        for (pane_id, instance_id, updates, agent_probe, cwd_probe, notice_probe, view_source) in
            std::mem::take(&mut self.startup_terminals)
        {
            monitor_terminal(TerminalMonitor {
                pane_id,
                instance_id,
                updates,
                agent_probe,
                cwd_probe,
                notice_probe,
                view_source,
                state: Arc::clone(&self.state),
                lifecycle: Arc::clone(&self.lifecycle),
            });
        }

        tracing::info!(
            version = env!("CARGO_PKG_VERSION"),
            pid = std::process::id(),
            tcp = self.tcp.is_some(),
            "Server started"
        );
        let mut run_result = Ok(());
        let mut next_client_id = 1_u64;
        let mut agent_expiry_due = Instant::now();
        'accept: while !self.stop.load(Ordering::Acquire) {
            if Instant::now() >= agent_expiry_due {
                self.state
                    .lock()
                    .expect("server state lock poisoned")
                    .expire_agent_operations(Instant::now());
                agent_expiry_due = Instant::now() + Duration::from_millis(50);
            }
            let mut accepted = false;
            for listener in std::iter::once(&self.local).chain(self.tcp.as_ref()) {
                match listener.accept() {
                    Ok(stream) => {
                        accepted = true;
                        let client_id = next_client_id;
                        next_client_id = next_client_id.wrapping_add(1);
                        let state = Arc::clone(&self.state);
                        let stop = Arc::clone(&self.stop);
                        let lifecycle = Arc::clone(&self.lifecycle);
                        thread::spawn(move || {
                            handle_client(stream, client_id, state, stop, lifecycle)
                        });
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                    Err(error) => {
                        run_result = Err(error);
                        break 'accept;
                    }
                }
            }
            if !accepted {
                thread::sleep(ACCEPT_POLL);
            }
        }

        tracing::info!("Server stopping");
        self.lifecycle.begin_stop();
        self.stop.store(true, Ordering::Release);
        self.lifecycle.wait_for_operations();
        let mut terminals = {
            let mut state = self.state.lock().expect("server state lock poisoned");
            state.stop_agent_waits();
            clipboard_image::remove(
                std::mem::take(&mut state.staged_images)
                    .into_values()
                    .flatten(),
            );
            state.terminal_instances.clear();
            for runtime in state.terminals.values() {
                runtime.prepare_agent_shutdown();
            }
            std::mem::take(&mut state.terminals)
        };
        let mut terminal_result = Ok(());
        let mut final_cwds = Vec::new();
        let mut final_resumes = Vec::new();
        for (&pane_id, runtime) in &mut terminals {
            let cwd_probe = runtime.cwd_probe();
            let before_close = cwd_probe.observe();
            if let Err(error) = runtime.close() {
                tracing::warn!(
                    "failed to close Terminal for Pane {}: {error}",
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
            if let Some(resume) = runtime.agent_resume() {
                final_resumes.push((pane_id, resume));
            }
        }
        let mut persistence = {
            let mut state = self.state.lock().expect("server state lock poisoned");
            state.record_terminal_cwds(final_cwds);
            for (pane_id, resume) in final_resumes {
                state.record_agent_resume(pane_id, resume);
            }
            state.persistence.take()
        };
        let persistence_result = persistence
            .as_mut()
            .map_or(Ok(()), SnapshotPersistence::shutdown);
        if let Err(error) = &persistence_result {
            tracing::error!("final Session Snapshot flush failed: {error}");
        }
        drop(persistence);
        drop(terminals);
        let cleanup_result = self
            .local
            .cleanup()
            .and(self.tcp.as_ref().map_or(Ok(()), EndpointListener::cleanup));
        if let Err(error) = &cleanup_result {
            tracing::warn!("endpoint cleanup failed: {error}");
        }
        tracing::info!("Server stopped");
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
    #[cfg(test)]
    bootstrap_captures: std::sync::atomic::AtomicUsize,
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
    agent_control: AgentControl,
    terminal_titles: std::collections::HashMap<PaneId, String>,
    /// BEL attention is controller-only because PTY focus has one authoritative owner.
    pending_terminal_bells: std::collections::HashSet<PaneId>,
    workspace_git: std::collections::HashMap<WorkspaceId, WorkspaceGit>,
    /// Recomputes `workspace_git` when files change; `None` until the state has its shared
    /// handle, and in unit tests.
    git_watcher: Option<GitWatcher>,
    workspace_git_scanned_at: std::collections::HashMap<WorkspaceId, Instant>,
    /// HEAD fingerprints at the last rediscovery, shared by every Pane of the Workspace.
    workspace_git_heads: std::collections::HashMap<WorkspaceId, GitFingerprint>,
    worktree_root: Option<PathBuf>,
    active_controller: Option<u64>,
    focused_terminal: Option<PaneId>,
    events: std::collections::VecDeque<SequencedEvent>,
    subscribers: std::collections::HashMap<u64, ClientSubscriber>,
    /// Live TCP peers by client id, so a revocation can drop their connections.
    tcp_peers: std::collections::HashMap<u64, (crate::noise::PublicKey, EndpointStream)>,
    /// Clipboard images staged for each Client (ADR 0012); removed with the Client.
    staged_images: std::collections::HashMap<u64, Vec<PathBuf>>,
    settings_write: Arc<Mutex<()>>,
    persistence: Option<SnapshotPersistence>,
    settings: ServerSettings,
    config_path: Option<PathBuf>,
    /// This Server's endpoint as Panes see it in `CONDR_SOCKET_PATH`.
    socket_path: String,
    started_at: Instant,
}

/// What every new shell is started with: the configured program plus the Server identity
/// its Pane environment carries. Cloned out of the state for spawns that run unlocked.
#[derive(Clone, Debug, Default)]
pub(super) struct ShellLaunch {
    shell: String,
    socket_path: String,
}

impl ShellLaunch {
    pub(super) fn shell(&self) -> Option<&str> {
        Some(self.shell.as_str())
    }

    pub(super) fn pane_environment(&self, pane_id: PaneId) -> PaneEnvironment {
        PaneEnvironment {
            pane_id,
            socket_path: self.socket_path.clone(),
        }
    }
}

struct ClientSubscriber {
    writer: ClientWriter,
    terminal_baselines: std::collections::HashMap<PaneId, Arc<TerminalView>>,
    pending_terminals: std::collections::HashSet<PaneId>,
    render_generation: u64,
    bootstrap_pending: bool,
    deferred_reliable: std::collections::VecDeque<Vec<u8>>,
    deferred_clipboard: Option<Vec<u8>>,
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
    settings: ServerSettings,
}

struct BootstrapTerminalCapture {
    pane_id: PaneId,
    view: Arc<TerminalView>,
    exited: bool,
    title: Option<String>,
    attention: bool,
}

impl BootstrapCapture {
    fn materialize(self) -> SessionBootstrap {
        SessionBootstrap {
            server_id: self.server_id,
            runtime_epoch: self.runtime_epoch,
            session_id: self.session_id,
            sequence: self.sequence,
            snapshot: self.snapshot,
            settings: self.settings,
            terminals: self
                .terminals
                .into_iter()
                .map(|terminal| PaneTerminalSnapshot {
                    pane_id: terminal.pane_id,
                    view: Arc::unwrap_or_clone(terminal.view),
                    exited: terminal.exited,
                    title: terminal.title,
                    attention: terminal.attention,
                })
                .collect(),
            agents: self.agents,
            workspace_git: self.workspace_git,
            zoomed_panes: self.zoomed_panes,
        }
    }
}

struct SequencedEvent {
    sequence: u64,
    event: SessionEvent,
}

impl RuntimeState {
    #[cfg(test)]
    fn new(socket_path: &std::path::Path) -> Self {
        Self::with_session(
            socket_path,
            Session::new(),
            None,
            ServerSettings::default(),
            None,
        )
    }

    fn with_session(
        socket_path: &std::path::Path,
        session: Session,
        persistence: Option<SnapshotPersistence>,
        settings: ServerSettings,
        config_path: Option<PathBuf>,
    ) -> Self {
        Self {
            #[cfg(test)]
            bootstrap_captures: std::sync::atomic::AtomicUsize::new(0),
            server_id: ServerId(stable_endpoint_id(socket_path)),
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
            agent_control: AgentControl::default(),
            terminal_titles: std::collections::HashMap::new(),
            pending_terminal_bells: std::collections::HashSet::new(),
            workspace_git: std::collections::HashMap::new(),
            git_watcher: None,
            workspace_git_scanned_at: std::collections::HashMap::new(),
            workspace_git_heads: std::collections::HashMap::new(),
            worktree_root: default_worktree_root(),
            active_controller: None,
            focused_terminal: None,
            events: std::collections::VecDeque::new(),
            subscribers: std::collections::HashMap::new(),
            tcp_peers: std::collections::HashMap::new(),
            staged_images: std::collections::HashMap::new(),
            settings_write: Arc::new(Mutex::new(())),
            persistence,
            settings,
            config_path,
            socket_path: socket_path.to_string_lossy().into_owned(),
            started_at: Instant::now(),
        }
    }

    pub(super) fn shell_launch(&self) -> ShellLaunch {
        ShellLaunch {
            shell: self.settings.shell.clone(),
            socket_path: self.socket_path.clone(),
        }
    }

    /// Publishes a shell preference after its serialized disk transaction succeeds.
    fn set_shell(&mut self, shell: &str, origin: Option<(u64, &ClientWriter)>) -> bool {
        let shell = shell.trim();
        if self.settings.shell == shell {
            return false;
        }
        self.settings.shell = shell.to_owned();
        self.publish_event(
            SessionEvent::ServerSettingsChanged {
                settings: self.settings.clone(),
            },
            origin,
        )
    }
}

#[cfg(test)]
mod tests;
