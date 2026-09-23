//! The first exchange on every connection, frozen so that any two builds of Condr can
//! finish it and name why they cannot go on (ADR 0027, 0028).
//!
//! Frozen means field numbers in `proto/condr/v1/handshake.proto`: these types may gain
//! fields, which an older peer ignores, and never renumber one. Everything after `Welcome`
//! belongs to `ClientMessage`/`ServerMessage`; the protocol range decides whether the two
//! sides go on.

use super::{MIN_CLIENT_PROTOCOL, MIN_SERVER_PROTOCOL, PROTOCOL_VERSION, ServerId, SessionId};
use std::fmt;

/// The Client's first frame. `Credential` and `Tunnel` precede `Hello` on a Peer-to-peer
/// connection from an unknown Device (ADR 0026) and a local tunnel request (ADR 0025).
#[derive(Clone, Debug, Eq, PartialEq)]
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Hello {
    pub protocol: u32,
    /// [`crate::build_identity`] of the Client binary.
    pub build: String,
    pub client_name: String,
    /// The oldest Server protocol this Client works with; `0` from a peer that predates
    /// the field.
    pub min_server_protocol: u32,
}

impl Hello {
    pub fn new(client_name: impl Into<String>) -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            build: crate::build_identity().to_owned(),
            client_name: client_name.into(),
            min_server_protocol: MIN_SERVER_PROTOCOL,
        }
    }
}

/// The Server's only answer to a handshake. With a `refusal` the connection closes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Welcome {
    pub protocol: u32,
    /// [`crate::build_identity`] of the Server binary.
    pub build: String,
    pub server_id: ServerId,
    /// Always `SessionId(1)`: one Server serves one Session (ADR 0013). Kept so the
    /// Session-addressed messages after the handshake need no other source for it.
    pub session_id: SessionId,
    pub refusal: Option<Refusal>,
    /// The oldest Client protocol this Server works with.
    pub min_client_protocol: u32,
}

impl Welcome {
    /// A Server's answer with its own protocol range.
    pub fn new(server_id: ServerId, session_id: SessionId, refusal: Option<Refusal>) -> Self {
        Self {
            protocol: PROTOCOL_VERSION,
            build: crate::build_identity().to_owned(),
            server_id,
            session_id,
            refusal,
            min_client_protocol: MIN_CLIENT_PROTOCOL,
        }
    }
}

/// Why a Server closes a connection after reading its handshake. Members are only added.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Refusal {
    /// One side's protocol is below the other's minimum; all four are in `Hello`/`Welcome`.
    IncompatibleProtocol,
    NotAuthorized(String),
    PairingFailed(String),
    DeviceRevoked,
    /// The first frame did not decode as a `ClientHandshake`.
    Malformed(String),
    /// A local Server asked to tunnel to a Peer-to-peer Device (ADR 0025) could not reach it.
    TunnelFailed(String),
    /// A reason from a newer Server that this build does not know.
    Unknown,
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
            Self::Unknown => formatter
                .write_str("refused for a reason this build does not know; update this Device"),
        }
    }
}
