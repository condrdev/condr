//! Versioned, transport-independent server/client protocol.
//!
//! The server owns the runtime. A client starts with [`ClientMessage::Hello`],
//! receives a [`ServerMessage::Bootstrap`], and then consumes ordered events.
//! The same framing works over local IPC and a future TCP/SSH transport.

use std::fmt;
use std::io::{self, Cursor, Read, Write};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::{PaneId, SessionSnapshot, TerminalCommand, TerminalView};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_FRAME_SIZE: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct ServerId(pub u64);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct SessionId(pub u64);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub struct RuntimeEpoch(pub u128);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Hello {
    pub version: u32,
    pub client_name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ClientMessage {
    Hello(Hello),
    SnapshotRequest {
        session_id: SessionId,
    },
    Subscribe {
        session_id: SessionId,
        after_sequence: u64,
    },
    Ping {
        server_id: ServerId,
        nonce: u64,
    },
    AcquireControl {
        session_id: SessionId,
    },
    ReleaseControl {
        session_id: SessionId,
    },
    CreateWorkspace {
        server_id: ServerId,
        session_id: SessionId,
        root_directory: PathBuf,
    },
    Terminal {
        server_id: ServerId,
        session_id: SessionId,
        pane_id: PaneId,
        command: TerminalCommand,
    },
    StopServer {
        server_id: ServerId,
    },
    Detach,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionBootstrap {
    pub server_id: ServerId,
    pub runtime_epoch: RuntimeEpoch,
    pub session_id: SessionId,
    pub sequence: u64,
    pub snapshot: SessionSnapshot,
    pub terminals: Vec<PaneTerminalSnapshot>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PaneTerminalSnapshot {
    pub pane_id: PaneId,
    pub view: TerminalView,
    pub exited: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SessionEvent {
    SnapshotChanged,
    TerminalChanged { pane_id: PaneId, view: TerminalView },
    TerminalExited { pane_id: PaneId, view: TerminalView },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ServerMessage {
    Welcome {
        version: u32,
        server_id: ServerId,
        runtime_epoch: RuntimeEpoch,
        session_id: SessionId,
        error: Option<String>,
    },
    Bootstrap(SessionBootstrap),
    Subscribed {
        server_id: ServerId,
        session_id: SessionId,
        sequence: u64,
    },
    Event {
        server_id: ServerId,
        session_id: SessionId,
        sequence: u64,
        event: SessionEvent,
    },
    Pong {
        server_id: ServerId,
        nonce: u64,
        sequence: u64,
    },
    ControlGranted {
        server_id: ServerId,
        session_id: SessionId,
    },
    ControlReleased {
        server_id: ServerId,
        session_id: SessionId,
    },
    ControlDenied {
        server_id: ServerId,
        session_id: SessionId,
        reason: String,
    },
    TerminalCopied {
        pane_id: PaneId,
        text: Option<String>,
    },
    ServerStopping,
    Error {
        message: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VersionCheck {
    Compatible,
    Incompatible(String),
}

pub fn check_version(version: u32) -> VersionCheck {
    if version == PROTOCOL_VERSION {
        VersionCheck::Compatible
    } else {
        VersionCheck::Incompatible(format!(
            "protocol version {version} is incompatible with server version {PROTOCOL_VERSION}"
        ))
    }
}

#[derive(Debug)]
pub enum FramingError {
    Io(io::Error),
    UnexpectedEof,
    Oversized { claimed: usize, max: usize },
    Codec(String),
}

impl fmt::Display for FramingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "I/O error: {error}"),
            Self::UnexpectedEof => formatter.write_str("unexpected end of stream"),
            Self::Oversized { claimed, max } => {
                write!(formatter, "frame size {claimed} exceeds maximum {max}")
            }
            Self::Codec(error) => write!(formatter, "protocol codec error: {error}"),
        }
    }
}

impl std::error::Error for FramingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for FramingError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn write_message<W, M>(writer: &mut W, message: &M) -> Result<(), FramingError>
where
    W: Write,
    M: Serialize,
{
    let payload =
        bincode::serialize(message).map_err(|error| FramingError::Codec(error.to_string()))?;
    if payload.len() > MAX_FRAME_SIZE {
        return Err(FramingError::Oversized {
            claimed: payload.len(),
            max: MAX_FRAME_SIZE,
        });
    }
    let length = u32::try_from(payload.len()).map_err(|_| {
        FramingError::Codec("payload length does not fit in the frame prefix".into())
    })?;
    writer.write_all(&length.to_le_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

pub fn read_message<R, M>(reader: &mut R) -> Result<M, FramingError>
where
    R: Read,
    M: for<'de> Deserialize<'de>,
{
    let mut prefix = [0; 4];
    read_exact_or_eof(reader, &mut prefix)?;
    let claimed = u32::from_le_bytes(prefix) as usize;
    if claimed > MAX_FRAME_SIZE {
        return Err(FramingError::Oversized {
            claimed,
            max: MAX_FRAME_SIZE,
        });
    }

    let mut payload = vec![0; claimed];
    read_exact_or_eof(reader, &mut payload)?;
    let mut cursor = Cursor::new(payload.as_slice());
    let message = bincode::deserialize_from(&mut cursor)
        .map_err(|error| FramingError::Codec(error.to_string()))?;
    if cursor.position() != claimed as u64 {
        return Err(FramingError::Codec("trailing bytes are not allowed".into()));
    }
    Ok(message)
}

fn read_exact_or_eof<R: Read>(reader: &mut R, buffer: &mut [u8]) -> Result<(), FramingError> {
    reader.read_exact(buffer).map_err(|error| {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            FramingError::UnexpectedEof
        } else {
            FramingError::Io(error)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_round_trip_uses_length_prefix() {
        let message = ClientMessage::Hello(Hello {
            version: PROTOCOL_VERSION,
            client_name: "test".into(),
        });
        let mut bytes = Vec::new();
        write_message(&mut bytes, &message).unwrap();
        assert_eq!(
            u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize,
            bytes.len() - 4
        );
        assert_eq!(
            read_message::<_, ClientMessage>(&mut bytes.as_slice()).unwrap(),
            message
        );
    }

    #[test]
    fn incompatible_versions_are_rejected() {
        assert!(matches!(
            check_version(PROTOCOL_VERSION + 1),
            VersionCheck::Incompatible(_)
        ));
    }

    #[test]
    fn oversized_frame_is_rejected_before_allocation() {
        let bytes = ((MAX_FRAME_SIZE as u32) + 1).to_le_bytes();
        assert!(matches!(
            read_message::<_, ClientMessage>(&mut bytes.as_slice()),
            Err(FramingError::Oversized { .. })
        ));
    }
}
