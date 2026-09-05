use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use condr_core::protocol::{
    BootstrapAssembler, BootstrapBatch, BootstrapHeader, BootstrapRecord, ClientMessage,
    FramingError, Hello, LayoutCommand, LayoutResult, MAX_BOOTSTRAP_BATCHES,
    MAX_BOOTSTRAP_TOTAL_SIZE, MAX_CHUNK_PAYLOAD_SIZE, MAX_FRAME_SIZE, PROTOCOL_VERSION,
    PaneAgentSnapshot, PaneTerminalFrame, PaneTerminalMetadata, PaneTerminalSnapshot, RuntimeEpoch,
    ServerId, ServerMessage, ServerSettings, SessionBootstrap, SessionEvent, SessionId,
    SessionOverview, TerminalFrameBatch, TerminalFrameChunk, VersionCheck, WorkspaceGitSnapshot,
    check_version, encode_bootstrap_record, encode_pane_terminal_frame,
};
use condr_core::{
    AgentSnapshot, GitHeadFingerprint, GitRepository, PaneEnvironment, PaneId, Session,
    TerminalAgentProbe, TerminalCommand, TerminalCwdProbe, TerminalHyperlinkBudget,
    TerminalNoticeBatch, TerminalNoticeProbe, TerminalRuntime, TerminalSize, TerminalUpdate,
    TerminalView, TerminalViewFrame, TerminalViewSource, WorkspaceId, create_worktree,
    default_worktree_root, discover_repository, open_worktree, remove_worktree,
    validate_worktree_removal,
};

use crate::client_writer::{ClientWriteItem, ClientWriter, ReliableSendError};
use crate::endpoint::{
    Endpoint, EndpointListener, EndpointStream, TcpEndpoint, default_socket_path,
};
use crate::noise::{ServerIdentity, StaticKey};
use crate::persistence::{SnapshotLoad, SnapshotPersistence};

mod agents;
mod bootstrap;
mod client;
mod layout;
mod local;
mod terminal_monitor;

use agents::AgentControl;
use bootstrap::*;
use client::*;
use layout::*;
#[cfg(test)]
use local::snapshot_path_for_endpoint;
pub use local::{
    connected_devices, ensure_local_server, ensure_server, probe_server, revoke_devices,
    stop_server,
};
use local::{default_snapshot_path, runtime_epoch, stable_endpoint_id};
use terminal_monitor::*;

const ACCEPT_POLL: Duration = Duration::from_millis(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(4);
const STOP_ACK_TIMEOUT: Duration = Duration::from_secs(1);
const EVENT_HISTORY_LIMIT: usize = 256;
const MAX_SHELL_SETTING_BYTES: usize = 4 * 1024;
/// Process cwd reads are rate-limited behind terminal output; agent detection has its
/// own cadence inside `TerminalAgentProbe`.
const CWD_SCAN_INTERVAL: Duration = Duration::from_millis(500);
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

/// One Server per host: it always answers on a private local socket, and on a TCP
/// address as well when `[server] listen` is configured. Both reach the same Session.
#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub socket_path: PathBuf,
    pub listen: Option<std::net::SocketAddr>,
    snapshot_path: Option<PathBuf>,
    /// The Server's own `config.toml`; `None` keeps settings in memory only.
    config_path: Option<PathBuf>,
    /// The key the TCP listener answers with; `None` loads the host identity at bind time.
    identity: Option<Arc<ServerIdentity>>,
    /// The device key `ephemeral_tcp` authorized, so tests can connect over TCP.
    test_device: Option<StaticKey>,
}

impl Default for ServerConfig {
    /// The host's Server: default socket, and the TCP address `config.toml` names, if any.
    fn default() -> Self {
        let config_path = condr_core::config_directory().map(|root| root.join("config.toml"));
        let socket_path = default_socket_path();
        Self {
            listen: load_listen(config_path.as_deref()),
            snapshot_path: default_snapshot_path(&socket_path),
            socket_path,
            config_path,
            identity: None,
            test_device: None,
        }
    }
}

/// `[server] listen` from `config.toml`: the TCP address the Server also answers on.
/// Absent, blank or malformed means no TCP listener.
pub fn load_listen(path: Option<&std::path::Path>) -> Option<std::net::SocketAddr> {
    load_server_setting(path?, "listen")?.trim().parse().ok()
}

/// Persists `[server] listen`, or removes it for `None`.
pub fn save_listen(path: &std::path::Path, listen: Option<std::net::SocketAddr>) -> io::Result<()> {
    save_server_setting(
        path,
        "listen",
        listen.map(|address| toml_edit::value(address.to_string())),
    )
}

fn load_server_setting(path: &std::path::Path, key: &str) -> Option<String> {
    crate::persistence::read_config_text(path)
        .ok()??
        .parse::<toml::Table>()
        .ok()?
        .get("server")?
        .get(key)?
        .as_str()
        .map(str::to_owned)
}

/// `[server.terminal] shell` from the Server's `config.toml`, blank when unset or the
/// file is missing or malformed. Read once at startup; hand edits need a restart.
fn load_shell(path: Option<&std::path::Path>) -> String {
    let Some(path) = path else {
        return String::new();
    };
    crate::persistence::read_config_text(path)
        .ok()
        .flatten()
        .and_then(|text| text.parse::<toml::Table>().ok())
        .and_then(|root| {
            root.get("server")?
                .get("terminal")?
                .get("shell")?
                .as_str()
                .map(|shell| shell.trim().to_owned())
        })
        .unwrap_or_default()
}

/// Writes `[server.terminal] shell` back, keeping the rest of the hand-editable file
/// (other keys, comments, formatting) as it was.
fn save_shell(path: &std::path::Path, shell: &str) -> io::Result<()> {
    save_setting(
        path,
        &["server", "terminal"],
        "shell",
        Some(toml_edit::value(shell)),
    )
}

fn save_server_setting(
    path: &std::path::Path,
    key: &str,
    value: Option<toml_edit::Item>,
) -> io::Result<()> {
    save_setting(path, &["server"], key, value)
}

/// Sets or, for `None`, removes one key under `tables` in `config.toml`, leaving every
/// other key, comment and line as written.
fn save_setting(
    path: &std::path::Path,
    tables: &[&str],
    key: &str,
    value: Option<toml_edit::Item>,
) -> io::Result<()> {
    crate::persistence::update_config_values(path, tables, [(key, value)])
}

impl ServerConfig {
    /// The host's Server on a specific socket, with the default snapshot for that socket.
    pub fn at_socket(socket_path: impl Into<PathBuf>) -> Self {
        let socket_path = socket_path.into();
        Self {
            listen: None,
            snapshot_path: default_snapshot_path(&socket_path),
            socket_path,
            config_path: condr_core::config_directory().map(|root| root.join("config.toml")),
            identity: None,
            test_device: None,
        }
    }

    /// A Server on `socket_path` with no persistence and no TCP listener.
    pub fn ephemeral(socket_path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
            listen: None,
            snapshot_path: None,
            config_path: None,
            identity: None,
            test_device: None,
        }
    }

    /// An ephemeral Server that also listens on `address` with a fresh identity accepting
    /// exactly one fresh device key. [`BoundServer::endpoint`] then connects as that device.
    pub fn ephemeral_tcp(address: std::net::SocketAddr) -> io::Result<Self> {
        let client_key = StaticKey::generate()?;
        let identity = ServerIdentity::ephemeral()?.with_authorized(client_key.public());
        let socket_path = std::env::temp_dir().join(format!(
            "condr-tcp-{}-{}.sock",
            std::process::id(),
            runtime_epoch()
        ));
        Ok(Self {
            socket_path,
            listen: Some(address),
            snapshot_path: None,
            config_path: None,
            identity: Some(Arc::new(identity)),
            test_device: Some(client_key),
        })
    }

    pub fn with_listen(mut self, address: std::net::SocketAddr) -> Self {
        self.listen = Some(address);
        self
    }

    pub fn with_identity(mut self, identity: ServerIdentity) -> Self {
        self.identity = Some(Arc::new(identity));
        self
    }

    pub fn local_endpoint(&self) -> Endpoint {
        Endpoint::local(&self.socket_path)
    }

    pub fn with_snapshot_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.snapshot_path = Some(path.into());
        self
    }

    pub fn with_config_path(mut self, path: impl Into<PathBuf>) -> Self {
        self.config_path = Some(path.into());
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
    bootstrap: Option<SessionBootstrap>,
    overview: SessionOverview,
    next_request_id: u64,
}

impl ClientConnection {
    pub fn connect(endpoint: &Endpoint, client_name: impl Into<String>) -> io::Result<Self> {
        Self::connect_with_state(endpoint, client_name.into(), true)
    }

    /// Connects for CLI inspection and mutations, without requesting terminal views.
    pub fn connect_overview(
        endpoint: &Endpoint,
        client_name: impl Into<String>,
    ) -> io::Result<Self> {
        Self::connect_with_state(endpoint, client_name.into(), false)
    }

    fn connect_with_state(
        endpoint: &Endpoint,
        client_name: String,
        terminal_views: bool,
    ) -> io::Result<Self> {
        match Self::handshake(endpoint.connect()?, client_name.clone(), terminal_views) {
            // The Server may already have recorded this device from an earlier attempt
            // that broke before the Client completed its initial query; it expects the zero
            // pre-shared key, so a refused invite is retried as a paired device.
            Err(error)
                if error.kind() == io::ErrorKind::PermissionDenied
                    && matches!(endpoint, Endpoint::Tcp(tcp) if tcp.invite.is_some()) =>
            {
                let Endpoint::Tcp(tcp) = endpoint else {
                    unreachable!()
                };
                let paired = Endpoint::tcp(tcp.clone().without_invite());
                Self::handshake(paired.connect()?, client_name, terminal_views).map_err(|_| error)
            }
            result => result,
        }
    }

    fn handshake(
        stream: EndpointStream,
        client_name: impl Into<String>,
        terminal_views: bool,
    ) -> io::Result<Self> {
        let (mut stream, _, session_id) = Self::welcome(stream, client_name)?;
        let request = if terminal_views {
            ClientMessage::SnapshotRequest { session_id }
        } else {
            ClientMessage::OverviewRequest { session_id }
        };
        condr_core::protocol::write_message(&mut stream, &request)
            .map_err(|error| io::Error::other(error.to_string()))?;
        let (bootstrap, overview) = if terminal_views {
            let bootstrap = Self::read_bootstrap(&mut stream)?;
            let overview = SessionOverview::from(&bootstrap);
            (Some(bootstrap), overview)
        } else {
            let response = condr_core::protocol::read_message(&mut stream)
                .map_err(|error| io::Error::other(error.to_string()))?;
            let ServerMessage::Overview(mut overview) = response else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unexpected overview response: {response:?}"),
                ));
            };
            let response = condr_core::protocol::read_message(&mut stream)
                .map_err(|error| io::Error::other(error.to_string()))?;
            match response {
                ServerMessage::OverviewTerminals {
                    server_id,
                    session_id,
                    terminals,
                } if server_id == overview.server_id && session_id == overview.session_id => {
                    overview.terminals = terminals;
                }
                response => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("unexpected overview terminal response: {response:?}"),
                    ));
                }
            }
            (None, overview)
        };
        stream.set_handshake_timeout(None)?;
        Ok(Self {
            stream,
            bootstrap,
            overview,
            next_request_id: 1,
        })
    }

    /// The last authoritative structure, updated from query responses and layout events.
    pub fn session(&self) -> io::Result<Session> {
        Session::restore(self.overview.snapshot.clone())
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
    }

    pub fn overview(&self) -> &SessionOverview {
        &self.overview
    }

    /// Sends terminal input and confirms the Server took it. Input itself gets no reply,
    /// so a Ping follows it: an `Error` arriving before the Pong belongs to the input.
    pub fn terminal(&mut self, pane_id: PaneId, command: TerminalCommand) -> io::Result<()> {
        let server_id = self.overview.server_id;
        let nonce = self.next_request_id;
        self.next_request_id += 1;
        condr_core::protocol::write_message(
            &mut self.stream,
            &ClientMessage::Terminal {
                server_id,
                session_id: self.overview.session_id,
                pane_id,
                command,
            },
        )
        .map_err(|error| io::Error::other(error.to_string()))?;
        condr_core::protocol::write_message(
            &mut self.stream,
            &ClientMessage::Ping { server_id, nonce },
        )
        .map_err(|error| io::Error::other(error.to_string()))?;
        loop {
            match condr_core::protocol::read_message(&mut self.stream)
                .map_err(|error| io::Error::other(error.to_string()))?
            {
                ServerMessage::Pong {
                    nonce: answered, ..
                } if answered == nonce => return Ok(()),
                ServerMessage::Error { message } => return Err(io::Error::other(message)),
                ServerMessage::ControlDenied { reason, .. } => {
                    return Err(io::Error::new(io::ErrorKind::PermissionDenied, reason));
                }
                _ => {}
            }
        }
    }

    /// The last `lines` rows of a Pane, scrollback included.
    pub fn read_pane(&mut self, pane_id: PaneId, lines: u32) -> io::Result<String> {
        condr_core::protocol::write_message(
            &mut self.stream,
            &ClientMessage::ReadPane {
                server_id: self.overview.server_id,
                session_id: self.overview.session_id,
                pane_id,
                lines,
            },
        )
        .map_err(|error| io::Error::other(error.to_string()))?;
        loop {
            match condr_core::protocol::read_message(&mut self.stream)
                .map_err(|error| io::Error::other(error.to_string()))?
            {
                ServerMessage::PaneText {
                    pane_id: read,
                    text,
                } if read == pane_id => {
                    return Ok(text);
                }
                ServerMessage::Error { message } => return Err(io::Error::other(message)),
                _ => {}
            }
        }
    }

    pub fn agent(
        &mut self,
        command: condr_core::protocol::AgentCommand,
    ) -> io::Result<Result<condr_core::protocol::AgentResponse, condr_core::protocol::AgentError>>
    {
        condr_core::protocol::write_message(
            &mut self.stream,
            &ClientMessage::Agent {
                server_id: self.overview.server_id,
                session_id: self.overview.session_id,
                command,
            },
        )
        .map_err(|error| io::Error::other(error.to_string()))?;
        loop {
            match condr_core::protocol::read_message(&mut self.stream)
                .map_err(|error| io::Error::other(error.to_string()))?
            {
                ServerMessage::AgentResult { result } => return Ok(result),
                ServerMessage::Error { message } => return Err(io::Error::other(message)),
                ServerMessage::SubscriptionRejected { reason, .. } => {
                    return Err(io::Error::other(reason));
                }
                ServerMessage::ServerStopping => {
                    return Err(io::Error::other("Server is stopping"));
                }
                _ => {}
            }
        }
    }

    /// Returns the command's actual created IDs, retaining its authoritative layout event.
    pub fn layout(&mut self, command: LayoutCommand) -> io::Result<Result<LayoutResult, String>> {
        let request_id = self.next_request_id;
        self.next_request_id += 1;
        condr_core::protocol::write_message(
            &mut self.stream,
            &ClientMessage::Layout {
                server_id: self.overview.server_id,
                session_id: self.overview.session_id,
                request_id,
                command,
            },
        )
        .map_err(|error| io::Error::other(error.to_string()))?;
        loop {
            match condr_core::protocol::read_message(&mut self.stream)
                .map_err(|error| io::Error::other(error.to_string()))?
            {
                ServerMessage::LayoutApplied {
                    request_id: applied,
                    result,
                    ..
                } if applied == request_id => return Ok(Ok(result)),
                ServerMessage::Event {
                    sequence,
                    event:
                        SessionEvent::LayoutChanged {
                            snapshot,
                            zoomed_panes,
                        },
                    ..
                } => {
                    self.overview.sequence = sequence;
                    self.overview.snapshot = snapshot;
                    self.overview.zoomed_panes = zoomed_panes;
                }
                ServerMessage::LayoutRejected {
                    request_id: rejected,
                    reason,
                    ..
                } if rejected == request_id => return Ok(Err(reason)),
                ServerMessage::Error { message } => return Err(io::Error::other(message)),
                _ => {}
            }
        }
    }

    /// Hello/Welcome only: enough to know a compatible Server answers, without pulling
    /// its whole Bootstrap. The local probe uses this so startup does not transfer every
    /// terminal view once for the probe and again for the real connection.
    pub(super) fn welcome(
        mut stream: EndpointStream,
        client_name: impl Into<String>,
    ) -> io::Result<(EndpointStream, ServerId, SessionId)> {
        // A refused handshake surfaces as an I/O error with its own kind; keep it so
        // callers can tell "not authorized" from a framing problem.
        let io_error = |error: FramingError| match error {
            FramingError::Io(error) => error,
            other => io::Error::other(other.to_string()),
        };
        stream.set_handshake_timeout(Some(HANDSHAKE_TIMEOUT))?;
        condr_core::protocol::write_message(
            &mut stream,
            &ClientMessage::Hello(Hello {
                version: PROTOCOL_VERSION,
                client_name: client_name.into(),
            }),
        )
        .map_err(io_error)?;
        let welcome: ServerMessage =
            condr_core::protocol::read_message(&mut stream).map_err(io_error)?;
        let (server_id, session_id) = match welcome {
            ServerMessage::Welcome {
                server_id,
                session_id,
                error: None,
                ..
            } => (server_id, session_id),
            ServerMessage::Welcome {
                error: Some(error), ..
            } => return Err(io::Error::new(io::ErrorKind::InvalidData, error)),
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("unexpected welcome response: {other:?}"),
                ));
            }
        };
        Ok((stream, server_id, session_id))
    }

    /// Reads one complete Bootstrap: the header followed by its batches.
    fn read_bootstrap(stream: &mut EndpointStream) -> io::Result<SessionBootstrap> {
        let bootstrap = match condr_core::protocol::read_message(stream)
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
            let batch = match condr_core::protocol::read_message(stream)
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
        assembler
            .finish()
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
    }

    /// Initial Bootstrap, present only for `connect`, which requests terminal views.
    pub fn bootstrap(&self) -> Option<&SessionBootstrap> {
        self.bootstrap.as_ref()
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
        Ok(Self {
            local,
            tcp,
            endpoint,
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

        self.lifecycle.begin_stop();
        self.stop.store(true, Ordering::Release);
        self.lifecycle.wait_for_operations();
        let mut terminals = {
            let mut state = self.state.lock().expect("server state lock poisoned");
            state.stop_agent_waits();
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
        let cleanup_result = self
            .local
            .cleanup()
            .and(self.tcp.as_ref().map_or(Ok(()), EndpointListener::cleanup));
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
    workspace_git: std::collections::HashMap<WorkspaceId, GitRepository>,
    workspace_git_scanned_at: std::collections::HashMap<WorkspaceId, Instant>,
    /// HEAD fingerprints at the last rediscovery, shared by every Pane of the Workspace.
    workspace_git_heads: std::collections::HashMap<WorkspaceId, GitHeadFingerprint>,
    worktree_root: Option<PathBuf>,
    active_controller: Option<u64>,
    focused_terminal: Option<PaneId>,
    events: std::collections::VecDeque<SequencedEvent>,
    subscribers: std::collections::HashMap<u64, ClientSubscriber>,
    /// Live TCP peers by client id, so a revocation can drop their connections.
    tcp_peers: std::collections::HashMap<u64, (crate::noise::PublicKey, EndpointStream)>,
    settings_write: Arc<Mutex<()>>,
    persistence: Option<SnapshotPersistence>,
    settings: ServerSettings,
    config_path: Option<PathBuf>,
    /// This Server's endpoint as Panes see it in `CONDR_SOCKET_PATH`.
    socket_path: String,
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

struct LatestTerminalView {
    view: Arc<TerminalView>,
    hyperlinks: TerminalHyperlinkBudget,
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
            workspace_git_scanned_at: std::collections::HashMap::new(),
            workspace_git_heads: std::collections::HashMap::new(),
            worktree_root: default_worktree_root(),
            active_controller: None,
            focused_terminal: None,
            events: std::collections::VecDeque::new(),
            subscribers: std::collections::HashMap::new(),
            tcp_peers: std::collections::HashMap::new(),
            settings_write: Arc::new(Mutex::new(())),
            persistence,
            settings,
            config_path,
            socket_path: socket_path.to_string_lossy().into_owned(),
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

    fn clear_controller_terminal_state(&mut self) {
        for runtime in self.terminals.values() {
            let _ = runtime.release_mouse();
            // The selection belongs to the controller (ADR 0008); a successor must not
            // inherit or copy it.
            let _ = runtime.execute(TerminalCommand::Select(None));
        }
        self.clear_terminal_focus();
        for pane_id in std::mem::take(&mut self.pending_terminal_bells) {
            self.publish_background(SessionEvent::TerminalAttentionChanged {
                pane_id,
                attention: false,
            });
        }
    }

    fn clear_terminal_focus(&mut self) {
        if let Some(pane_id) = self.focused_terminal.take()
            && let Some(runtime) = self.terminals.get(&pane_id)
        {
            let _ = runtime.execute(TerminalCommand::Focus(false));
        }
    }

    fn recover(
        socket_path: &std::path::Path,
        snapshot_path: Option<PathBuf>,
        config_path: Option<PathBuf>,
    ) -> io::Result<(Self, Vec<StartedTerminal>)> {
        let settings = ServerSettings {
            shell: load_shell(config_path.as_deref()),
            default_shell: condr_core::default_shell_program(),
        };
        let launch = ShellLaunch {
            shell: settings.shell.clone(),
            socket_path: socket_path.to_string_lossy().into_owned(),
        };
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
                launch.shell(),
                Some(&launch.pane_environment(pane_id)),
            ) {
                Ok(runtime) => Some((runtime, requested_cwd.to_path_buf())),
                Err(error) if requested_cwd != workspace_root.as_path() => {
                    eprintln!(
                        "condr-server: fresh shell failed for Pane {} in saved cwd {}; retrying Workspace Root {}: {error}",
                        pane_id.as_u64(),
                        requested_cwd.display(),
                        workspace_root.display()
                    );
                    match TerminalRuntime::spawn_shell(
                        &workspace_root,
                        TerminalSize::new(24, 80),
                        launch.shell(),
                        Some(&launch.pane_environment(pane_id)),
                    ) {
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

        let mut state =
            Self::with_session(socket_path, session, persistence, settings, config_path);
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
        self.forget_agent_control(pane_id);
        let probe = runtime
            .agent_probe()
            .expect("new Terminal has an agent probe");
        let cwd_probe = runtime.cwd_probe();
        let notice_probe = runtime.notice_probe();
        let view_source = runtime.view_source();
        self.clear_terminal_title(pane_id);
        self.clear_terminal_attention(pane_id);
        let mut view = runtime.view();
        let hyperlinks = TerminalHyperlinkBudget::new(&mut view);
        self.terminal_views.insert(
            pane_id,
            LatestTerminalView {
                view: Arc::new(view),
                hyperlinks,
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
        (
            pane_id,
            instance_id,
            updates,
            probe,
            cwd_probe,
            notice_probe,
            view_source,
        )
    }

    /// Forgets a Terminal's reported title and tells subscribers, if there was one.
    fn clear_terminal_title(&mut self, pane_id: PaneId) {
        if self.terminal_titles.remove(&pane_id).is_some() {
            self.publish_background(SessionEvent::TerminalTitleChanged {
                pane_id,
                title: None,
            });
        }
    }

    fn record_terminal_focus(&mut self, pane_id: PaneId, focused: bool) {
        if focused {
            self.focused_terminal = Some(pane_id);
            self.clear_terminal_attention(pane_id);
        } else if self.focused_terminal == Some(pane_id) {
            self.focused_terminal = None;
        }
    }

    fn clear_terminal_attention(&mut self, pane_id: PaneId) {
        if self.pending_terminal_bells.remove(&pane_id) {
            self.publish_background(SessionEvent::TerminalAttentionChanged {
                pane_id,
                attention: false,
            });
        }
    }

    fn restore_exited_terminal(&mut self, pane_id: PaneId, runtime: TerminalRuntime) -> bool {
        if self.session.pane(pane_id).is_none() || self.terminals.contains_key(&pane_id) {
            return false;
        }
        let view = runtime.view();
        self.terminal_instances.remove(&pane_id);
        self.forget_agent_control(pane_id);
        self.closing_terminals.remove(&pane_id);
        self.terminals.insert(pane_id, runtime);
        self.publish_terminal(pane_id, TerminalViewFrame::Full(view));
        if self.exited_terminals.insert(pane_id) {
            self.publish_background(SessionEvent::TerminalExited { pane_id });
        }
        self.clear_terminal_title(pane_id);
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
            LayoutCommand::CreateTab { workspace_id, .. } => self
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
                .map(|(&pane_id, &agent)| PaneAgentSnapshot { pane_id, agent })
                .collect(),
            workspace_git: self
                .workspace_git
                .iter()
                .map(|(&workspace_id, repository)| workspace_git_snapshot(workspace_id, repository))
                .collect(),
            zoomed_panes: self.zoomed_panes(),
        }
    }

    fn zoomed_panes(&self) -> Vec<PaneId> {
        self.session
            .workspaces()
            .iter()
            .flat_map(|workspace| workspace.tabs())
            .filter_map(|tab| tab.zoomed_pane_id())
            .collect()
    }

    /// The event every structural change publishes: the new Snapshot and zoom state, so
    /// clients apply it in place rather than re-fetching a Bootstrap.
    fn layout_changed_event(&self) -> SessionEvent {
        SessionEvent::LayoutChanged {
            snapshot: self.session.snapshot(),
            zoomed_panes: self.zoomed_panes(),
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
        result: LayoutResult,
    ) -> bool {
        let event = self.layout_changed_event();
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
                result,
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
        // The event is already committed; a lagged origin merely leaves the subscriber set
        // and recovers through its Bootstrap. Only a dead writer closes the connection.
        let origin_result = origin.map(|(_, writer)| writer.send_reliable(data.clone()));
        let origin_lost = origin_result.is_some_and(|result| result.is_err());
        self.subscribers.retain(|client_id, subscriber| {
            if origin.is_some_and(|(origin_client_id, _)| *client_id == origin_client_id) {
                !origin_lost
            } else if subscriber.bootstrap_pending {
                subscriber.deferred_reliable.push_back(data.clone());
                true
            } else {
                subscriber.writer.send_reliable(data.clone()).is_ok()
            }
        });
        origin_result == Some(Err(ReliableSendError::Disconnected))
    }

    /// Fans out the latest non-replayed clipboard state to every attached client.
    fn broadcast_clipboard(&mut self, pane_id: PaneId, text: String) {
        let Ok(data) = frame_message(&ServerMessage::TerminalClipboard { pane_id, text }) else {
            return;
        };
        self.subscribers.retain(|_, subscriber| {
            if subscriber.bootstrap_pending {
                subscriber.deferred_clipboard = Some(data.clone());
                true
            } else {
                subscriber.writer.send_clipboard(data.clone()).is_ok()
            }
        });
    }

    fn publish_terminal(&mut self, pane_id: PaneId, frame: TerminalViewFrame) -> Option<Vec<u64>> {
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
        // Clipboard state is neither in the Bootstrap nor in event history, so a copy that is
        // still unsent must survive the fence (ADR 0007); newer copies keep replacing it and
        // go out after the Bootstrap. Only lag recovery drops it (`resume_after_lag`).
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
                    deferred_clipboard: None,
                });
                true
            }
        }
    }

    fn finish_bootstrap(
        &mut self,
        client_id: u64,
        framed: FramedBootstrap,
    ) -> Result<(), ReliableSendError> {
        let Some(subscriber) = self.subscribers.get_mut(&client_id) else {
            return Err(ReliableSendError::Disconnected);
        };
        subscriber.writer.clear_render();
        subscriber.terminal_baselines = framed.baselines.into_iter().collect();
        subscriber.writer.send_reliable_batch(framed.frames)?;
        while let Some(data) = subscriber.deferred_reliable.pop_front() {
            subscriber.writer.send_reliable(data)?;
        }
        if let Some(data) = subscriber.deferred_clipboard.take() {
            subscriber.writer.send_clipboard(data)?;
        }
        subscriber.bootstrap_pending = false;
        subscriber.render_generation = subscriber.render_generation.wrapping_add(1);
        Ok(())
    }

    fn abort_bootstrap(&mut self, client_id: u64) {
        if let Some(subscriber) = self.subscribers.get_mut(&client_id) {
            subscriber.bootstrap_pending = false;
            subscriber.deferred_reliable.clear();
            if let Some(data) = subscriber.deferred_clipboard.take() {
                let _ = subscriber.writer.send_clipboard(data);
            }
            subscriber.pending_terminals.clear();
            subscriber.render_generation = subscriber.render_generation.wrapping_add(1);
        }
    }
}

#[cfg(test)]
mod tests;
