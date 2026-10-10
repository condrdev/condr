//! The two transports a Companion reaches a Server over, which it dials itself (ADR 0040).

use crate::{
    Connector, NoiseStream, P2pEndpoint, P2pNode, P2pStream, SLOW_HANDSHAKE_TIMEOUT, ServerStream,
    TcpEndpoint,
};
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::time::Duration;

/// An address the Companion dials: TCP directly, Peer-to-peer through its own endpoint.
#[derive(Clone)]
pub enum RemoteEndpoint {
    Tcp(TcpEndpoint),
    P2p {
        node: Arc<P2pNode>,
        address: P2pEndpoint,
    },
}

pub enum RemoteStream {
    Tcp(NoiseStream),
    P2p(P2pStream),
}

impl Connector for RemoteEndpoint {
    type Stream = RemoteStream;

    fn connect(&self) -> io::Result<RemoteStream> {
        match self {
            Self::Tcp(tcp) => tcp.connect().map(RemoteStream::Tcp),
            Self::P2p { node, address } => node
                .dial(address.device, address.invite.as_ref())
                .map(RemoteStream::P2p),
        }
    }

    /// Only TCP retries: a Peer-to-peer Device redeems its invite in a `Credential` frame
    /// the Server ignores once it knows the device.
    fn without_invite(&self) -> Option<Self> {
        match self {
            Self::Tcp(tcp) if tcp.invite.is_some() => Some(Self::Tcp(tcp.clone().without_invite())),
            Self::Tcp(_) | Self::P2p { .. } => None,
        }
    }
}

impl Read for RemoteStream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.read(buffer),
            Self::P2p(stream) => stream.read(buffer),
        }
    }
}

impl Write for RemoteStream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Tcp(stream) => stream.write(buffer),
            Self::P2p(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Tcp(stream) => stream.flush(),
            Self::P2p(stream) => stream.flush(),
        }
    }
}

impl ServerStream for RemoteStream {
    fn try_clone(&self) -> io::Result<Self> {
        match self {
            Self::Tcp(stream) => stream.try_clone().map(Self::Tcp),
            Self::P2p(stream) => stream.try_clone().map(Self::P2p),
        }
    }

    fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        match self {
            Self::Tcp(stream) => stream.socket().set_read_timeout(timeout),
            Self::P2p(stream) => {
                stream.set_read_timeout(timeout);
                Ok(())
            }
        }
    }

    fn shutdown(&self) -> io::Result<()> {
        match self {
            Self::Tcp(stream) => stream.shutdown(),
            Self::P2p(stream) => stream.shutdown(),
        }
    }

    /// A relayed path over cellular answers slower than a TCP connection on a LAN.
    fn welcome_timeout(&self) -> Duration {
        match self {
            Self::Tcp(_) => crate::HANDSHAKE_TIMEOUT,
            Self::P2p(_) => SLOW_HANDSHAKE_TIMEOUT,
        }
    }
}
