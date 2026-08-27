use std::io;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use murmur_core::protocol::{
    ClientMessage, FramingError, Hello, PROTOCOL_VERSION, PaneTerminalSnapshot, RuntimeEpoch,
    ServerId, ServerMessage, SessionBootstrap, SessionEvent, SessionId, VersionCheck,
    check_version,
};
use murmur_core::{
    PaneId, Session, TerminalCommand, TerminalRuntime, TerminalSize, TerminalUpdate,
};

use crate::endpoint::{Endpoint, EndpointListener, EndpointStream, default_socket_path};

const ACCEPT_POLL: Duration = Duration::from_millis(10);
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(4);
const EVENT_HISTORY_LIMIT: usize = 256;

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
            state: Arc::new(Mutex::new(state)),
            next_client_id: Arc::new(AtomicU64::new(1)),
        })
    }

    pub fn handle(&self) -> ServerHandle {
        ServerHandle {
            stop: Arc::clone(&self.stop),
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
                    thread::spawn(move || handle_client(stream, client_id, state, stop));
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(ACCEPT_POLL);
                }
                Err(error) => {
                    self.stop.store(true, Ordering::Release);
                    let _ = self.endpoint.cleanup();
                    return Err(error);
                }
            }
        }
        self.state
            .lock()
            .expect("server state lock poisoned")
            .terminals
            .clear();
        self.endpoint.cleanup()
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
    active_controller: Option<u64>,
    events: std::collections::VecDeque<SequencedEvent>,
    subscribers: std::collections::HashMap<u64, mpsc::Sender<ServerMessage>>,
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
        }
    }

    fn publish_snapshot_change(
        &mut self,
        origin_client_id: u64,
        origin: &mpsc::Sender<ServerMessage>,
    ) -> bool {
        let event = SessionEvent::SnapshotChanged;
        self.publish_event(event, Some((origin_client_id, origin)))
    }

    fn publish_background(&mut self, event: SessionEvent) {
        self.publish_event(event, None);
    }

    fn publish_event(
        &mut self,
        event: SessionEvent,
        origin: Option<(u64, &mpsc::Sender<ServerMessage>)>,
    ) -> bool {
        // ponytail: bounded full-view replay; add cell damage events only after profiling.
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
        let origin_failed = origin.is_some_and(|(_, sender)| sender.send(message.clone()).is_err());
        self.subscribers.retain(|client_id, sender| {
            if origin.is_some_and(|(origin_client_id, _)| *client_id == origin_client_id) {
                !origin_failed
            } else {
                sender.send(message.clone()).is_ok()
            }
        });
        origin_failed
    }
}

fn handle_client(
    mut stream: EndpointStream,
    client_id: u64,
    state: Arc<Mutex<RuntimeState>>,
    stop: Arc<AtomicBool>,
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
    let (outbound, outbound_rx) = mpsc::channel();
    let writer = thread::spawn(move || {
        while let Ok(message) = outbound_rx.recv() {
            if send_message(&mut writer_stream, &message).is_err() {
                break;
            }
        }
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
        let should_close = match message {
            ClientMessage::SnapshotRequest { session_id } => {
                let response = {
                    let state = state.lock().expect("server state lock poisoned");
                    if session_id != state.session_id {
                        ServerMessage::Error {
                            message: "unknown Session".into(),
                        }
                    } else {
                        ServerMessage::Bootstrap(state.bootstrap())
                    }
                };
                queue_message(&outbound, response)
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
                    state.subscribers.insert(client_id, outbound.clone());
                }
                let failed = responses
                    .into_iter()
                    .any(|response| queue_message(&outbound, response));
                if failed {
                    state.subscribers.remove(&client_id);
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
            ClientMessage::CreateWorkspace {
                server_id,
                session_id,
                root_directory,
            } => {
                let mut state = state.lock().expect("server state lock poisoned");
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
                } else if state.active_controller != Some(client_id) {
                    queue_message(
                        &outbound,
                        ServerMessage::ControlDenied {
                            server_id: state.server_id,
                            session_id,
                            reason: "acquire Session control before mutating layout".into(),
                        },
                    )
                } else {
                    let mut candidate = Session::restore(state.session.snapshot())
                        .expect("live Session always restores from its own Snapshot");
                    let workspace_id = candidate.create_workspace(root_directory);
                    let pane = candidate
                        .workspace(workspace_id)
                        .expect("new Workspace exists")
                        .active_tab()
                        .focused_pane();
                    let pane_id = pane.id();
                    let cwd = pane.cwd().expect("new Pane has a cwd").to_path_buf();
                    match TerminalRuntime::spawn_shell(cwd, TerminalSize::new(24, 80)) {
                        Ok(mut runtime) => {
                            let updates = runtime
                                .take_updates()
                                .expect("new Terminal update receiver exists");
                            state.session = candidate;
                            state.terminals.insert(pane_id, runtime);
                            started_terminal = Some((pane_id, updates));
                            state.publish_snapshot_change(client_id, &outbound)
                        }
                        Err(error) => queue_message(
                            &outbound,
                            ServerMessage::Error {
                                message: format!("failed to start terminal: {error}"),
                            },
                        ),
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
                } else if state.active_controller != Some(client_id) {
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
                    let is_copy = matches!(&command, TerminalCommand::Copy);
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
                    let _ = queue_message(&outbound, ServerMessage::ServerStopping);
                    stopping_server = true;
                    true
                }
            }
            ClientMessage::Detach => true,
            ClientMessage::Hello(Hello { .. }) => false,
        };
        if let Some((pane_id, updates)) = started_terminal {
            monitor_terminal(pane_id, updates, Arc::clone(&state));
        }
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
    state: Arc<Mutex<RuntimeState>>,
) {
    thread::spawn(move || {
        while let Ok(update) = updates.recv() {
            let mut state = state.lock().expect("server state lock poisoned");
            match update {
                TerminalUpdate::View(_) => {
                    let Some(view) = state.terminals.get(&pane_id).map(TerminalRuntime::view)
                    else {
                        break;
                    };
                    state.publish_background(SessionEvent::TerminalChanged { pane_id, view });
                }
                TerminalUpdate::Exited => {
                    let Some(view) = state.terminals.get_mut(&pane_id).map(|runtime| {
                        let _ = runtime.wait();
                        runtime.view()
                    }) else {
                        break;
                    };
                    state.exited_terminals.insert(pane_id);
                    state.publish_background(SessionEvent::TerminalExited { pane_id, view });
                    break;
                }
            }
        }
    });
}

fn queue_message(outbound: &mpsc::Sender<ServerMessage>, message: ServerMessage) -> bool {
    outbound.send(message).is_err()
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

    #[cfg(target_os = "linux")]
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
        murmur_core::protocol::write_message(
            &mut first,
            &ClientMessage::CreateWorkspace {
                server_id,
                session_id,
                root_directory: std::env::temp_dir(),
            },
        )
        .unwrap();
        wait_for_message(&mut first, |message| {
            matches!(
                message,
                ServerMessage::Event {
                    event: SessionEvent::SnapshotChanged,
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
        send_terminal(
            &mut first,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Text("printf 'murmur-pid=%s\\n' $$\r".into()),
        );
        wait_for_terminal_text(&mut first, pane_id, "murmur-pid=");
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

        send_terminal(
            &mut second,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Paste("printf reconnect-ok\n".into()),
        );
        wait_for_terminal_text(&mut second, pane_id, "reconnect-ok");
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
        wait_for_terminal_text(&mut second, pane_id, "10 40");
        send_terminal(
            &mut second,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Text(
                "i=1; while [ $i -le 20 ]; do echo line-$i; i=$((i+1)); done\r".into(),
            ),
        );
        wait_for_terminal_text(&mut second, pane_id, "line-20");
        thread::sleep(Duration::from_millis(50));
        send_terminal(
            &mut second,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Scroll(TerminalScroll::Top),
        );
        let scrolled = wait_for_terminal(&mut second, pane_id, |view| view.display_offset > 0);
        let expected = (0..4)
            .filter_map(|column| scrolled.cell(0, column))
            .map(|cell| cell.text.as_str())
            .collect::<String>();
        send_terminal(
            &mut second,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Select {
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
            },
        );
        wait_for_terminal(&mut second, pane_id, |view| {
            view.cell(0, 0).is_some_and(|cell| cell.selected)
        });
        send_terminal(
            &mut second,
            server_id,
            session_id,
            pane_id,
            TerminalCommand::Copy,
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

    #[cfg(target_os = "linux")]
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

    #[cfg(target_os = "linux")]
    fn wait_for_terminal_text(
        stream: &mut EndpointStream,
        pane_id: PaneId,
        needle: &str,
    ) -> murmur_core::TerminalView {
        wait_for_terminal(stream, pane_id, |view| view_text(view).contains(needle))
    }

    #[cfg(target_os = "linux")]
    fn wait_for_terminal(
        stream: &mut EndpointStream,
        pane_id: PaneId,
        predicate: impl Fn(&murmur_core::TerminalView) -> bool,
    ) -> murmur_core::TerminalView {
        match wait_for_message(stream, |message| {
            matches!(
                message,
                ServerMessage::Event {
                    event: SessionEvent::TerminalChanged { pane_id: event_pane, view },
                    ..
                } if *event_pane == pane_id && predicate(view)
            )
        }) {
            ServerMessage::Event {
                event: SessionEvent::TerminalChanged { view, .. },
                ..
            } => view,
            _ => unreachable!(),
        }
    }

    #[cfg(target_os = "linux")]
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

    #[cfg(target_os = "linux")]
    fn read_server(stream: &mut EndpointStream) -> ServerMessage {
        murmur_core::protocol::read_message(stream).unwrap()
    }

    #[cfg(target_os = "linux")]
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
            &ClientMessage::CreateWorkspace {
                server_id,
                session_id,
                root_directory: std::env::temp_dir(),
            },
        )
        .unwrap();
        assert!(matches!(
            murmur_core::protocol::read_message::<_, ServerMessage>(&mut first).unwrap(),
            ServerMessage::Event {
                server_id: event_server,
                session_id: event_session,
                sequence: 1,
                event: SessionEvent::SnapshotChanged,
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
                    event: SessionEvent::SnapshotChanged,
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
                &ClientMessage::CreateWorkspace {
                    server_id,
                    session_id,
                    root_directory: std::env::temp_dir(),
                },
            )
            .unwrap();
            match murmur_core::protocol::read_message::<_, ServerMessage>(&mut controller).unwrap()
            {
                ServerMessage::Event {
                    sequence,
                    event: SessionEvent::SnapshotChanged,
                    ..
                } => snapshot_sequences.push(sequence),
                other => panic!("unexpected mutation response: {other:?}"),
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
                    if event == SessionEvent::SnapshotChanged {
                        replayed_snapshots.push(sequence);
                    }
                }
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
}
