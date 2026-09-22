mod local;
mod saved;
mod stream;

use crate::noise::{DeviceKey, NoiseStream, PublicKey, Secret, ServerIdentity};
use crate::ssh::{SshEndpoint, SshStream};
#[cfg(windows)]
use atomicwrites::{AtomicFile, DisallowOverwrite};
use interprocess::local_socket::traits::Listener as _;
use local::{LocalStream, connect_local};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub(crate) use local::acquire_local_bind_lock;
pub use local::{EndpointListener, default_socket_path};
pub use saved::{SavedServer, load_saved_servers, save_saved_servers};
pub use stream::{ConnectionCancellation, EndpointStream};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Endpoint {
    Local(PathBuf),
    Tcp(TcpEndpoint),
    Ssh(SshEndpoint),
    P2p(P2pEndpoint),
}

/// A Device reached Peer-to-peer by its key alone (ADR 0026). The Client does not dial
/// it itself: its own local Server is the machine's Peer-to-peer endpoint, so `connect`
/// opens the local socket and asks for a `Tunnel` (ADR 0025).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct P2pEndpoint {
    pub device: PublicKey,
    pub invite: Option<Secret>,
}

impl P2pEndpoint {
    /// Parses `p2p://<id>[.<invite>]`, as printed by an invite.
    pub fn parse(text: &str) -> io::Result<Self> {
        let invalid = |reason: &str| io::Error::new(io::ErrorKind::InvalidInput, reason.to_owned());
        let credentials = text
            .trim()
            .strip_prefix("p2p://")
            .ok_or_else(|| invalid("expected p2p://<id>[.<invite>]"))?
            .trim_end_matches('/');
        let (device, invite) = match credentials.split_once('.') {
            Some((device, invite)) => (device, Some(Secret::parse(invite)?)),
            None => (credentials, None),
        };
        Ok(Self {
            device: PublicKey::parse(device)?,
            invite,
        })
    }

    pub fn without_invite(mut self) -> Self {
        self.invite = None;
        self
    }

    /// Asks this machine's Server to dial the Device; the returned stream carries the
    /// remote Server's frames from its `Welcome` on, or one `Error` if the dial failed.
    /// Inside a Pane that is the Pane's Server; elsewhere the default Server, started
    /// when it is not running, as the GUI does (ADR 0025).
    fn connect(&self) -> io::Result<EndpointStream> {
        let local = match std::env::var(condr_core::PaneEnvironment::SOCKET_PATH) {
            Ok(value) => Endpoint::from_env_value(&value)?,
            Err(_) => crate::server::ensure_local_server()?,
        };
        let path = local.as_local_path().ok_or_else(|| {
            io::Error::other("this machine's Server has no local socket to tunnel through")
        })?;
        let mut stream = connect_local(path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "this machine's Server is unreachable at {}: {error}",
                    path.display()
                ),
            )
        })?;
        condr_core::protocol::write_message(
            &mut stream,
            &condr_core::protocol::ClientHandshake::Tunnel {
                device: *self.device.as_bytes(),
                invite: self.invite.as_ref().map(|invite| *invite.as_bytes()),
            },
        )
        .map_err(|error| io::Error::other(error.to_string()))?;
        Ok(EndpointStream::Tunnel(stream))
    }
}

/// Everything a Client needs to reach one TCP Server: where it listens, whose Device key
/// it must present, which device key to speak with, and the invite that pairs a device
/// the Server does not know yet. The Server binds with the same value, so its own
/// `server_key` and `client_key` are the host identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TcpEndpoint {
    /// A host name or IP address, resolved when connecting.
    pub host: String,
    pub port: u16,
    pub server_key: PublicKey,
    pub client_key: DeviceKey,
    pub invite: Option<Secret>,
}

impl TcpEndpoint {
    /// Parses `tcp://<server key>[.<invite>]@host:port`, as printed by an invite.
    pub fn parse(text: &str, client_key: DeviceKey) -> io::Result<Self> {
        let invalid = |reason: &str| io::Error::new(io::ErrorKind::InvalidInput, reason.to_owned());
        let (credentials, authority) = text
            .trim()
            .strip_prefix("tcp://")
            .ok_or_else(|| invalid("expected tcp://<server key>[.<invite>]@host:port"))?
            .rsplit_once('@')
            .ok_or_else(|| invalid("expected tcp://<server key>[.<invite>]@host:port"))?;
        let (host, port) = Self::split_authority(authority)?;
        let (server_key, invite) = match credentials.split_once('.') {
            Some((key, invite)) => (key, Some(Secret::parse(invite)?)),
            None => (credentials, None),
        };
        Ok(Self {
            host,
            port,
            server_key: PublicKey::parse(server_key)?,
            client_key,
            invite,
        })
    }

    /// Splits `host:port`; the host may be a name, an IPv4 address or a bracketed IPv6
    /// address.
    pub fn split_authority(authority: &str) -> io::Result<(String, u16)> {
        let invalid = |reason: &str| io::Error::new(io::ErrorKind::InvalidInput, reason.to_owned());
        let (host, port) = authority
            .trim()
            .rsplit_once(':')
            .ok_or_else(|| invalid("expected host:port"))?;
        let host = host.trim_start_matches('[').trim_end_matches(']').trim();
        if host.is_empty() {
            return Err(invalid("the host is empty"));
        }
        let port = port
            .parse::<u16>()
            .ok()
            .filter(|port| *port != 0)
            .ok_or_else(|| invalid("the port must be a number from 1 to 65535"))?;
        Ok((host.to_owned(), port))
    }

    /// A Server bound at `address`, as its own tests connect to it.
    pub fn at(address: SocketAddr, server_key: PublicKey, client_key: DeviceKey) -> Self {
        Self {
            host: address.ip().to_string(),
            port: address.port(),
            server_key,
            client_key,
            invite: None,
        }
    }

    /// `host:port`, with an IPv6 host in brackets.
    pub fn authority(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    pub fn without_invite(mut self) -> Self {
        self.invite = None;
        self
    }
}

impl Endpoint {
    /// Parse a remote address without requiring a TCP device key for SSH.
    pub fn parse(text: &str, client_key: Option<&DeviceKey>) -> io::Result<Self> {
        let text = text.trim();
        if text.starts_with("ssh://") {
            return SshEndpoint::parse(text).map(Self::Ssh);
        }
        if text.starts_with("p2p://") {
            return P2pEndpoint::parse(text).map(Self::P2p);
        }
        if text.starts_with("tcp://") {
            let key = client_key.ok_or_else(|| {
                io::Error::other("this device has no TCP key; see the startup error")
            })?;
            return TcpEndpoint::parse(text, key.clone()).map(Self::Tcp);
        }
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "use a tcp://, ssh:// or p2p:// address",
        ))
    }

    pub fn local(path: impl Into<PathBuf>) -> Self {
        Self::Local(path.into())
    }

    pub fn tcp(endpoint: TcpEndpoint) -> Self {
        Self::Tcp(endpoint)
    }

    pub fn connect(&self) -> io::Result<EndpointStream> {
        match self {
            Self::Local(path) => connect_local(path).map(EndpointStream::Local),
            Self::Tcp(tcp) => NoiseStream::initiator(
                connect_tcp(&tcp.host, tcp.port)?,
                &tcp.server_key,
                &tcp.client_key,
                tcp.invite.as_ref(),
            )
            .map(EndpointStream::Tcp),
            Self::Ssh(ssh) => ssh.connect().map(EndpointStream::Ssh),
            Self::P2p(p2p) => p2p.connect(),
        }
    }

    /// How a Pane program finds its own Server: `CONDR_SOCKET_PATH` is always the local
    /// socket or pipe path, since the Server and the Pane share a host.
    pub fn from_env_value(value: &str) -> io::Result<Self> {
        if value.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "empty endpoint",
            ));
        }
        Ok(Self::local(value))
    }

    /// A failed connect in words. A missing socket file and a refused TCP connect both
    /// mean nobody is listening; the raw OS text says neither that nor which endpoint.
    pub fn describe_connect_error(&self, error: &io::Error) -> String {
        if let Some(refused) = crate::Refused::from_error(error)
            && refused.refusal == condr_core::protocol::Refusal::IncompatibleProtocol
        {
            return format!(
                "the device at {self} runs Condr {} (protocol {}); this is Condr {} (protocol {})",
                refused.server_build,
                refused.server_protocol,
                condr_core::build_identity(),
                condr_core::protocol::PROTOCOL_VERSION
            );
        }
        if matches!(self, Self::Ssh(_) | Self::P2p(_)) {
            return format!("could not connect to {self}: {error}");
        }
        match error.kind() {
            io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused => {
                format!("nothing is listening at {self}")
            }
            io::ErrorKind::PermissionDenied => {
                format!("the device at {self} refused this connection: {error}")
            }
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => {
                format!("the device at {self} did not answer in time")
            }
            io::ErrorKind::HostUnreachable | io::ErrorKind::NetworkUnreachable => {
                format!("{self} is unreachable from this machine")
            }
            _ => format!("could not connect to {self}: {error}"),
        }
    }

    pub fn as_local_path(&self) -> Option<&Path> {
        match self {
            Self::Local(path) => Some(path),
            Self::Tcp(_) | Self::Ssh(_) | Self::P2p(_) => None,
        }
    }

    /// The host and port of a TCP endpoint, which identify a saved Server in the GUI.
    pub fn tcp_host_port(&self) -> Option<(&str, u16)> {
        match self {
            Self::Local(_) | Self::Ssh(_) | Self::P2p(_) => None,
            Self::Tcp(tcp) => Some((tcp.host.as_str(), tcp.port)),
        }
    }
}

/// The endpoint without key material: a local path, `tcp://host:port`, or an SSH URI.
impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Local(path) => write!(f, "{}", path.display()),
            Self::Tcp(tcp) => write!(f, "tcp://{}", tcp.authority()),
            Self::Ssh(ssh) => ssh.fmt(f),
            Self::P2p(p2p) => write!(f, "p2p://{}", p2p.device),
        }
    }
}

/// How long a TCP connect may take before the device is reported as not answering;
/// the OS default is several times longer and the GUI would sit on "Connecting…".
const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

fn connect_tcp(host: &str, port: u16) -> io::Result<TcpStream> {
    use std::net::ToSocketAddrs as _;
    // One budget for every address the name resolves to, so a dual-stack host with an
    // unreachable IPv6 route still fails within the timeout rather than N times it.
    let deadline = Instant::now() + TCP_CONNECT_TIMEOUT;
    let mut last_error = None;
    for address in (host, port).to_socket_addrs()? {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            last_error = Some(io::Error::new(io::ErrorKind::TimedOut, "connect timed out"));
            break;
        }
        match TcpStream::connect_timeout(&address, remaining) {
            Ok(stream) => {
                enable_keepalive(&stream)?;
                return Ok(stream);
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("{host} did not resolve to any address"),
        )
    }))
}

/// Without probes a peer that vanished (sleep, dropped link) leaves the reader blocked
/// forever: the GUI shows a live connection, the Server keeps the client and its control.
fn enable_keepalive(stream: &TcpStream) -> io::Result<()> {
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(Duration::from_secs(15))
        .with_interval(Duration::from_secs(5));
    socket2::SockRef::from(stream).set_tcp_keepalive(&keepalive)
}
