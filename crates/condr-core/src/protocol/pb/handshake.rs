use super::*;
use crate::protocol::{ClientHandshake, Hello, Refusal, ServerId, SessionId, Welcome};

fn key(bytes: Vec<u8>, field: &str) -> WireResult<[u8; 32]> {
    bytes
        .try_into()
        .map_err(|_| malformed(format!("{field} is not 32 bytes")))
}

impl From<&ClientHandshake> for super::ClientHandshake {
    fn from(handshake: &ClientHandshake) -> Self {
        use client_handshake::Kind;
        let kind = match handshake {
            ClientHandshake::Hello(hello) => Kind::Hello(super::Hello {
                protocol: hello.protocol,
                build: hello.build.clone(),
                client_name: hello.client_name.clone(),
                min_server_protocol: hello.min_server_protocol,
            }),
            ClientHandshake::Credential { invite } => Kind::Credential(Credential {
                invite: invite.to_vec(),
            }),
            ClientHandshake::Tunnel { device, invite } => Kind::Tunnel(Tunnel {
                device: device.to_vec(),
                invite: invite.map(|invite| invite.to_vec()),
            }),
        };
        Self { kind: Some(kind) }
    }
}

impl TryFrom<super::ClientHandshake> for ClientHandshake {
    type Error = WireError;

    fn try_from(handshake: super::ClientHandshake) -> WireResult<Self> {
        use client_handshake::Kind;
        Ok(match member(handshake.kind)? {
            Kind::Hello(hello) => Self::Hello(Hello {
                protocol: hello.protocol,
                build: hello.build,
                client_name: hello.client_name,
                min_server_protocol: hello.min_server_protocol,
            }),
            Kind::Credential(credential) => Self::Credential {
                invite: key(credential.invite, "invite")?,
            },
            Kind::Tunnel(tunnel) => Self::Tunnel {
                device: key(tunnel.device, "device")?,
                invite: tunnel
                    .invite
                    .map(|invite| key(invite, "invite"))
                    .transpose()?,
            },
        })
    }
}

impl From<&Welcome> for super::Welcome {
    fn from(welcome: &Welcome) -> Self {
        Self {
            protocol: welcome.protocol,
            build: welcome.build.clone(),
            server_id: welcome.server_id.0,
            session_id: welcome.session_id.0,
            refusal: welcome.refusal.as_ref().map(super::Refusal::from),
            min_client_protocol: welcome.min_client_protocol,
        }
    }
}

impl TryFrom<super::Welcome> for Welcome {
    type Error = WireError;

    fn try_from(welcome: super::Welcome) -> WireResult<Self> {
        Ok(Self {
            protocol: welcome.protocol,
            build: welcome.build,
            server_id: ServerId(welcome.server_id),
            session_id: SessionId(welcome.session_id),
            refusal: welcome.refusal.map(Refusal::from),
            min_client_protocol: welcome.min_client_protocol,
        })
    }
}

impl From<&Refusal> for super::Refusal {
    fn from(refusal: &Refusal) -> Self {
        use refusal::Reason;
        let reason = match refusal {
            Refusal::IncompatibleProtocol => Reason::IncompatibleProtocol(Empty {}),
            Refusal::NotAuthorized(reason) => Reason::NotAuthorized(reason.clone()),
            Refusal::PairingFailed(reason) => Reason::PairingFailed(reason.clone()),
            Refusal::DeviceRevoked => Reason::DeviceRevoked(Empty {}),
            Refusal::Malformed(reason) => Reason::Malformed(reason.clone()),
            Refusal::TunnelFailed(reason) => Reason::TunnelFailed(reason.clone()),
            // Never sent: a Server only refuses for reasons it knows.
            Refusal::Unknown => return Self { reason: None },
        };
        Self {
            reason: Some(reason),
        }
    }
}

impl From<super::Refusal> for Refusal {
    fn from(refusal: super::Refusal) -> Self {
        use refusal::Reason;
        match refusal.reason {
            Some(Reason::IncompatibleProtocol(_)) => Self::IncompatibleProtocol,
            Some(Reason::NotAuthorized(reason)) => Self::NotAuthorized(reason),
            Some(Reason::PairingFailed(reason)) => Self::PairingFailed(reason),
            Some(Reason::DeviceRevoked(_)) => Self::DeviceRevoked,
            Some(Reason::Malformed(reason)) => Self::Malformed(reason),
            Some(Reason::TunnelFailed(reason)) => Self::TunnelFailed(reason),
            None => Self::Unknown,
        }
    }
}
