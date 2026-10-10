//! What a Client does with a Server short of drawing it (ADR 0039): the Device key, both
//! ends of the TCP and Peer-to-peer transports, and the connection the GUI, the CLI and the
//! Companion share. `condr-server` adds the desktop's local socket, SSH and `Tunnel`.

mod connection;
mod key;
mod noise;
pub mod p2p;
mod remote;
mod tcp;

pub use connection::{
    Cancellation, ClientConnection, Connector, HANDSHAKE_TIMEOUT, Refused, SLOW_HANDSHAKE_TIMEOUT,
    ServerStream,
};
pub use key::{DeviceKey, PublicKey, Secret};
pub use noise::{Authority, NoiseStream};
pub use p2p::{P2pEndpoint, P2pNode, P2pStream};
pub use remote::{RemoteEndpoint, RemoteStream};
pub use tcp::{TcpEndpoint, enable_keepalive};
