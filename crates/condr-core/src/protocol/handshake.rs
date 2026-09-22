//! The first exchange on every connection, frozen so that any two builds of Condr can
//! finish it and name why they cannot go on (ADR 0027).
//!
//! bincode is positional: a field added or moved here would make an older peer misread
//! the frame instead of learning that it is incompatible. These types therefore never
//! change. `Refusal` may only grow at its end. Everything after `Welcome` belongs to
//! `ClientMessage`/`ServerMessage` and is covered by [`PROTOCOL_VERSION`].

use super::{PROTOCOL_VERSION, ServerId, SessionId};
use serde::{Deserialize, Serialize};
use std::fmt;

/// The Client's first frame. `Credential` and `Tunnel` precede `Hello` on a Peer-to-peer
/// connection from an unknown Device (ADR 0026) and a local tunnel request (ADR 0025).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ClientHandshake {
    Hello(Hello),
    Credential {
        invite: [u8; 32],
    },
    Tunnel {
        device: [u8; 32],
        invite: Option<[u8; 32]>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub protocol: u32,
    /// [`crate::build_identity`] of the Client binary.
    pub build: String,
    pub client_name: String,
}

impl Hello {
    pub fn new(client_name: impl Into<String>) -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            build: crate::build_identity(),
            client_name: client_name.into(),
        }
    }
}

/// The Server's only answer to a handshake. With a `refusal` the connection closes.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Welcome {
    pub protocol: u32,
    /// [`crate::build_identity`] of the Server binary.
    pub build: String,
    pub server_id: ServerId,
    /// Always `SessionId(1)`: one Server serves one Session (ADR 0013). Kept so the
    /// Session-addressed messages after the handshake need no other source for it.
    pub session_id: SessionId,
    pub refusal: Option<Refusal>,
}

/// Why a Server closes a connection after reading its handshake. Append only.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Refusal {
    /// The Client's `protocol` differs from the Server's; both are in `Hello`/`Welcome`.
    IncompatibleProtocol,
    NotAuthorized(String),
    PairingFailed(String),
    DeviceRevoked,
    /// The first frame did not decode as a `ClientHandshake`.
    Malformed(String),
    /// A local Server asked to tunnel to a Peer-to-peer Device (ADR 0025) could not reach it.
    TunnelFailed(String),
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IncompatibleProtocol => formatter.write_str("incompatible protocol version"),
            Self::NotAuthorized(reason) => write!(formatter, "not authorized; {reason}"),
            Self::PairingFailed(reason) => write!(formatter, "pairing failed: {reason}"),
            Self::DeviceRevoked => formatter.write_str("device revoked"),
            Self::Malformed(reason) => write!(formatter, "invalid handshake frame: {reason}"),
            Self::TunnelFailed(reason) => write!(formatter, "could not reach the device: {reason}"),
        }
    }
}
