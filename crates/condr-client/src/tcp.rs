//! A Server reached over TCP (ADR 0011): its address, whose key it must present, and the
//! dial that opens a Noise connection to it.

use crate::{DeviceKey, NoiseStream, PublicKey, Secret};
use std::io;
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

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

    /// Dials the Server and starts the Noise handshake, which runs on first use.
    pub fn connect(&self) -> io::Result<NoiseStream> {
        NoiseStream::initiator(
            connect_tcp(&self.host, self.port)?,
            &self.server_key,
            &self.client_key,
            self.invite.as_ref(),
        )
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
/// Both ends set it: the dial here, and the Server on every TCP connection it accepts.
pub fn enable_keepalive(stream: &TcpStream) -> io::Result<()> {
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(Duration::from_secs(15))
        .with_interval(Duration::from_secs(5));
    socket2::SockRef::from(stream).set_tcp_keepalive(&keepalive)
}
