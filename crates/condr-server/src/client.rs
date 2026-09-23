//! Client protocol operations shared by the GUI and CLI.

use crate::{ConnectionCancellation, Endpoint, EndpointStream};
use condr_core::protocol::{
    BootstrapAssembler, ClientHandshake, ClientMessage, FramingError, Hello, LayoutCommand,
    LayoutResult, PROTOCOL_VERSION, Refusal, ServerMessage, SessionBootstrap, SessionEvent,
    SessionOverview, Welcome,
};
use condr_core::{PaneId, Session, TerminalCommand};
use std::fmt;
use std::io;
use std::time::Duration;

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

pub struct ClientConnection {
    stream: EndpointStream,
    cancellation: ConnectionCancellation,
    bootstrap: Option<SessionBootstrap>,
    overview: SessionOverview,
    /// The Server's [`condr_core::build_identity`], from its `Welcome`.
    server_build: String,
    next_request_id: u64,
}

impl ClientConnection {
    pub fn connect(endpoint: &Endpoint, client_name: impl Into<String>) -> io::Result<Self> {
        Self::connect_cancellable(endpoint, client_name, ConnectionCancellation::default())
    }

    /// The caller owns cancellation from before SSH spawn through connected I/O.
    pub fn connect_cancellable(
        endpoint: &Endpoint,
        client_name: impl Into<String>,
        cancellation: ConnectionCancellation,
    ) -> io::Result<Self> {
        Self::connect_with_state(endpoint, client_name.into(), true, cancellation)
    }

    pub fn cancellation(&self) -> ConnectionCancellation {
        self.cancellation.clone()
    }

    /// Connects for CLI inspection and mutations, without requesting terminal views.
    pub fn connect_overview(
        endpoint: &Endpoint,
        client_name: impl Into<String>,
    ) -> io::Result<Self> {
        Self::connect_with_state(
            endpoint,
            client_name.into(),
            false,
            ConnectionCancellation::default(),
        )
    }

    fn connect_with_state(
        endpoint: &Endpoint,
        client_name: String,
        terminal_views: bool,
        cancellation: ConnectionCancellation,
    ) -> io::Result<Self> {
        let attempt = |endpoint: &Endpoint| {
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
            Err(error)
                if error.kind() == io::ErrorKind::PermissionDenied
                    && matches!(endpoint, Endpoint::Tcp(tcp) if tcp.invite.is_some()) =>
            {
                let Endpoint::Tcp(tcp) = endpoint else {
                    unreachable!()
                };
                let paired = Endpoint::tcp(tcp.clone().without_invite());
                attempt(&paired).map_err(|_| error)
            }
            result => result,
        }
    }

    fn handshake(
        stream: EndpointStream,
        client_name: impl Into<String>,
        terminal_views: bool,
        cancellation: ConnectionCancellation,
    ) -> io::Result<Self> {
        let (mut stream, welcome) = Self::welcome(stream, client_name)?;
        let session_id = welcome.session_id;
        let server_build = welcome.build;
        // Authentication is complete. Bootstrap may be large: bound inactivity,
        // not total transfer time, just as socket read timeouts do.
        stream.set_handshake_timeout(Some(crate::server::HANDSHAKE_TIMEOUT))?;
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
    ) -> io::Result<(EndpointStream, Welcome)> {
        // A refused handshake surfaces as an I/O error with its own kind; keep it so
        // callers can tell "not authorized" from a framing problem.
        let io_error = |error: FramingError| match error {
            FramingError::Io(error) => error,
            other => io::Error::other(other.to_string()),
        };
        // SSH establishes its encrypted session before the first protocol byte, and a
        // tunnelled Peer-to-peer dial looks the Device up, reaches the relay and punches
        // through before the remote answers.
        let timeout = if matches!(stream, EndpointStream::Ssh(_) | EndpointStream::Tunnel(_)) {
            Duration::from_secs(15)
        } else {
            crate::server::HANDSHAKE_TIMEOUT
        };
        stream.set_handshake_timeout(Some(timeout))?;
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
    pub fn server_build(&self) -> &str {
        &self.server_build
    }

    pub fn bootstrap(&self) -> Option<&SessionBootstrap> {
        self.bootstrap.as_ref()
    }

    pub fn into_stream(self) -> EndpointStream {
        self.stream
    }
}
