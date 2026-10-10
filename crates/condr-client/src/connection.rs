//! The Client's side of the protocol, shared by the GUI, the CLI and the Companion: the
//! handshake, the initial Bootstrap or overview, and the request/reply helpers the CLI
//! uses. The transport is the caller's, through [`ServerStream`] and [`Connector`].

use condr_core::protocol::{
    BootstrapAssembler, ClientHandshake, ClientMessage, FramingError, Hello, LayoutCommand,
    LayoutResult, PROTOCOL_VERSION, Refusal, ServerMessage, SessionBootstrap, SessionEvent,
    SessionOverview, Welcome,
};
use condr_core::{PaneId, Session, TerminalCommand};
use std::fmt;
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How long a Server may take to answer `Hello` on a fast transport, and how long either
/// side waits for the next byte of a handshake or initial Bootstrap.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(4);

/// How long a transport that sets itself up after the first byte may take to answer
/// `Hello`: SSH establishes its session, and a Peer-to-peer dial looks the Device up,
/// reaches the relay and punches through.
pub const SLOW_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// A connected byte stream to a Server, whatever carries it: the desktop's local socket,
/// SSH and tunnel, or the TCP and Peer-to-peer streams this crate opens.
pub trait ServerStream: Read + Write + Send + Sized + 'static {
    fn try_clone(&self) -> io::Result<Self>;
    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()>;
    /// Ends the connection for every clone, so a blocked reader sees end of stream; a
    /// stream that only closes on drop does nothing.
    fn shutdown(&self) -> io::Result<()>;
    /// How long the Server may take to answer `Hello` over this stream.
    fn welcome_timeout(&self) -> Duration {
        HANDSHAKE_TIMEOUT
    }
    /// Whether cancelling shuts the stream down; a local socket is left to `Detach`.
    fn cancel_by_shutdown(&self) -> bool {
        true
    }
}

/// An address a Client dials.
pub trait Connector {
    type Stream: ServerStream;
    fn connect(&self) -> io::Result<Self::Stream>;
    /// This address without the invite it carries, if it carries one. A Server may have
    /// recorded the device on an earlier attempt that broke before the Client finished, and
    /// then expects the zero pre-shared key (ADR 0011).
    fn without_invite(&self) -> Option<Self>
    where
        Self: Sized;
}

/// Cancels a pending or established remote connection from another thread, without
/// waiting for its reader or writer. Local connections still use protocol `Detach`.
pub struct Cancellation<S> {
    state: Arc<Mutex<CancellationState<S>>>,
}

struct CancellationState<S> {
    cancelled: bool,
    stream: Option<S>,
}

impl<S> Clone for Cancellation<S> {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
        }
    }
}

impl<S> Default for Cancellation<S> {
    fn default() -> Self {
        Self {
            state: Arc::new(Mutex::new(CancellationState {
                cancelled: false,
                stream: None,
            })),
        }
    }
}

impl<S: ServerStream> Cancellation<S> {
    pub fn cancel(&self) {
        let mut state = self.state.lock().unwrap();
        state.cancelled = true;
        if let Some(stream) = state.stream.take() {
            let _ = stream.shutdown();
        }
    }

    pub fn connect<C: Connector<Stream = S>>(&self, connector: &C) -> io::Result<S> {
        if self.state.lock().unwrap().cancelled {
            return Err(cancelled());
        }
        let stream = connector.connect()?;
        self.attach(&stream)?;
        Ok(stream)
    }

    /// Lets `cancel` reach `stream`, or shuts it down at once if cancelled already.
    pub fn attach(&self, stream: &S) -> io::Result<()> {
        let mut state = self.state.lock().unwrap();
        if state.cancelled {
            let _ = stream.shutdown();
            return Err(cancelled());
        }
        state.stream = if stream.cancel_by_shutdown() {
            Some(stream.try_clone()?)
        } else {
            None
        };
        Ok(())
    }

    pub fn clear(&self) {
        if let Some(stream) = self.state.lock().unwrap().stream.take() {
            let _ = stream.shutdown();
        }
    }
}

fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "connection cancelled")
}

/// A Server read this Client's `Hello` and closed the connection. Reached through
/// `io::Error::get_ref` so callers can tell the refusal, and the Server's build, apart
/// from a transport failure without matching on text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Refused {
    pub refusal: Refusal,
    pub server_protocol: u32,
    pub server_build: String,
}

impl Refused {
    fn kind(&self) -> io::ErrorKind {
        match self.refusal {
            Refusal::IncompatibleProtocol | Refusal::Malformed(_) | Refusal::Unknown => {
                io::ErrorKind::InvalidData
            }
            Refusal::NotAuthorized(_) | Refusal::PairingFailed(_) | Refusal::DeviceRevoked => {
                io::ErrorKind::PermissionDenied
            }
            Refusal::TunnelFailed(_) => io::ErrorKind::HostUnreachable,
        }
    }
}

impl fmt::Display for Refused {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.refusal {
            Refusal::IncompatibleProtocol => write!(
                formatter,
                "protocol version {PROTOCOL_VERSION} is incompatible with server version {}",
                self.server_protocol
            ),
            other => other.fmt(formatter),
        }
    }
}

impl std::error::Error for Refused {}

impl Refused {
    /// The refusal behind a connect error, if that is what it was.
    pub fn from_error(error: &io::Error) -> Option<&Self> {
        error.get_ref()?.downcast_ref()
    }
}

impl From<Refused> for io::Error {
    fn from(refused: Refused) -> Self {
        io::Error::new(refused.kind(), refused)
    }
}

pub struct ClientConnection<S> {
    stream: S,
    cancellation: Cancellation<S>,
    bootstrap: Option<SessionBootstrap>,
    overview: SessionOverview,
    /// The Server's [`condr_core::build_identity`], from its `Welcome`.
    server_build: String,
    next_request_id: u64,
}

impl<S: ServerStream> ClientConnection<S> {
    pub fn connect<C: Connector<Stream = S>>(
        endpoint: &C,
        client_name: impl Into<String>,
    ) -> io::Result<Self> {
        Self::connect_cancellable(endpoint, client_name, Cancellation::default())
    }

    /// The caller owns cancellation from before SSH spawn through connected I/O.
    pub fn connect_cancellable<C: Connector<Stream = S>>(
        endpoint: &C,
        client_name: impl Into<String>,
        cancellation: Cancellation<S>,
    ) -> io::Result<Self> {
        Self::connect_with_state(endpoint, client_name.into(), true, cancellation)
    }

    pub fn cancellation(&self) -> Cancellation<S> {
        self.cancellation.clone()
    }

    /// Connects for CLI inspection and mutations, without requesting terminal views.
    pub fn connect_overview<C: Connector<Stream = S>>(
        endpoint: &C,
        client_name: impl Into<String>,
    ) -> io::Result<Self> {
        Self::connect_with_state(endpoint, client_name.into(), false, Cancellation::default())
    }

    fn connect_with_state<C: Connector<Stream = S>>(
        endpoint: &C,
        client_name: String,
        terminal_views: bool,
        cancellation: Cancellation<S>,
    ) -> io::Result<Self> {
        let attempt = |endpoint: &C| {
            let stream = cancellation.connect(endpoint)?;
            let result = Self::handshake(
                stream,
                client_name.clone(),
                terminal_views,
                cancellation.clone(),
            );
            if result.is_err() {
                cancellation.clear();
            }
            result
        };
        match attempt(endpoint) {
            // The Server may already have recorded this device from an earlier attempt
            // that broke before the Client completed its initial query; it expects the zero
            // pre-shared key, so a refused invite is retried as a paired device.
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                match endpoint.without_invite() {
                    Some(paired) => attempt(&paired).map_err(|_| error),
                    None => Err(error),
                }
            }
            result => result,
        }
    }

    fn handshake(
        stream: S,
        client_name: impl Into<String>,
        terminal_views: bool,
        cancellation: Cancellation<S>,
    ) -> io::Result<Self> {
        let (mut stream, welcome) = Self::welcome(stream, client_name)?;
        let session_id = welcome.session_id;
        let server_build = welcome.build;
        // Authentication is complete. Bootstrap may be large: bound inactivity,
        // not total transfer time, just as socket read timeouts do.
        stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
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
        stream.set_read_timeout(None)?;
        Ok(Self {
            stream,
            cancellation,
            bootstrap,
            overview,
            server_build,
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
    pub fn welcome(mut stream: S, client_name: impl Into<String>) -> io::Result<(S, Welcome)> {
        // A refused handshake surfaces as an I/O error with its own kind; keep it so
        // callers can tell "not authorized" from a framing problem.
        let io_error = |error: FramingError| match error {
            FramingError::Io(error) => error,
            other => io::Error::other(other.to_string()),
        };
        stream.set_read_timeout(Some(stream.welcome_timeout()))?;
        condr_core::protocol::write_message(
            &mut stream,
            &ClientHandshake::Hello(Hello::new(client_name)),
        )
        .map_err(io_error)?;
        let welcome: Welcome = condr_core::protocol::read_message(&mut stream).map_err(io_error)?;
        if let Some(refusal) = welcome.refusal.clone() {
            return Err(Refused {
                refusal,
                server_protocol: welcome.protocol,
                server_build: welcome.build,
            }
            .into());
        }
        Ok((stream, welcome))
    }

    /// Reads one complete Bootstrap: the header followed by its batches.
    fn read_bootstrap(stream: &mut S) -> io::Result<SessionBootstrap> {
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
    pub fn server_build(&self) -> &str {
        &self.server_build
    }

    pub fn bootstrap(&self) -> Option<&SessionBootstrap> {
        self.bootstrap.as_ref()
    }

    pub fn into_stream(self) -> S {
        self.stream
    }
}
