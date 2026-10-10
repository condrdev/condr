mod local;
mod saved;
mod stream;

use crate::noise::{DeviceKey, NoiseStream, ServerIdentity};
use crate::ssh::{SshEndpoint, SshStream};
#[cfg(windows)]
use atomicwrites::{AtomicFile, DisallowOverwrite};
use interprocess::local_socket::traits::Listener as _;
use local::{LocalStream, connect_local};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::net::{SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(windows)]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

pub use condr_client::{P2pEndpoint, TcpEndpoint};
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
            Self::Tcp(tcp) => tcp.connect().map(EndpointStream::Tcp),
            Self::Ssh(ssh) => ssh.connect().map(EndpointStream::Ssh),
            Self::P2p(p2p) => tunnel(p2p),
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

impl condr_client::Connector for Endpoint {
    type Stream = EndpointStream;

    fn connect(&self) -> io::Result<EndpointStream> {
        Endpoint::connect(self)
    }

    /// Only TCP pairs with its invite as the pre-shared key; a Peer-to-peer invite rides a
    /// `Credential` frame the Server ignores once it knows the device.
    fn without_invite(&self) -> Option<Self> {
        match self {
            Self::Tcp(tcp) if tcp.invite.is_some() => Some(Self::Tcp(tcp.clone().without_invite())),
            Self::Local(_) | Self::Tcp(_) | Self::Ssh(_) | Self::P2p(_) => None,
        }
    }
}

/// Asks this machine's Server to dial the Device; the returned stream carries the
/// remote Server's frames from its `Welcome` on, or one `Error` if the dial failed.
/// Inside a Pane that is the Pane's Server; elsewhere the default Server, started
/// when it is not running, as the GUI does (ADR 0025).
fn tunnel(p2p: &P2pEndpoint) -> io::Result<EndpointStream> {
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
            device: *p2p.device.as_bytes(),
            invite: p2p.invite.as_ref().map(|invite| *invite.as_bytes()),
        },
    )
    .map_err(|error| io::Error::other(error.to_string()))?;
    Ok(EndpointStream::Tunnel(stream))
}
