//! The Stream connection's messages (ADR 0061). A Client opens it beside its terminal
//! connection with `ClientHandshake::Streams`; after `Welcome` every frame is one
//! [`StreamClientMessage`] or [`StreamServerMessage`]. Both wrap a [`StreamMessage`],
//! and each direction encodes only the members it may send.

use crate::WorkspaceId;
use relative_path::RelativePathBuf;
use std::fmt;

/// The bytes a Stream may have in flight in each direction, and its initial window.
pub const STREAM_WINDOW: u64 = 1024 * 1024;
/// The Streams one Stream connection holds at once; the next `Open` fails with
/// [`StreamError::TooManyStreams`]. With [`STREAM_WINDOW`] it bounds a receiver's buffers.
pub const MAX_STREAMS: usize = 64;
/// The most bytes one `Data` carries.
pub const MAX_STREAM_CHUNK: usize = 32 * 1024;
/// A frame on the Stream connection: a chunk and its header with room to spare.
pub const MAX_STREAM_FRAME_SIZE: usize = 64 * 1024;

/// What an `Open` asks the Server for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StreamTarget {
    /// A port on the Server's own loopback. Kept as sent, so the Server can refuse 0 and
    /// values above 65535 as [`StreamError::InvalidPort`].
    Forward {
        port: u32,
    },
    Download(StreamFile),
    Upload {
        file: StreamFile,
        replace: bool,
    },
    /// A target from a newer Client; the Server answers [`StreamError::Unsupported`].
    Unknown,
}

/// A file inside a Workspace's Root Directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamFile {
    pub workspace: WorkspaceId,
    pub path: RelativePathBuf,
}

/// Why a Stream did not open or ended early. Unknown values decode to `Other`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamError {
    ConnectionRefused,
    NotFound,
    AlreadyExists,
    NotAFile,
    InvalidPath,
    InvalidPort,
    WorkspaceGone,
    TooManyStreams,
    Io,
    Unsupported,
    Cancelled,
    Other,
}

impl fmt::Display for StreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ConnectionRefused => "nothing is listening on that port",
            Self::NotFound => "not found",
            Self::AlreadyExists => "already exists",
            Self::NotAFile => "not a regular file",
            Self::InvalidPath => "invalid path",
            Self::InvalidPort => "invalid port",
            Self::WorkspaceGone => "the Workspace is gone",
            Self::TooManyStreams => "too many open streams",
            Self::Io => "I/O error",
            Self::Unsupported => "this Server does not support that stream; update it",
            Self::Cancelled => "cancelled",
            Self::Other => "failed",
        })
    }
}

/// One frame of the Stream connection, in either direction. `Open` is the Client's
/// alone, `Opened`, `OpenFailed` and `Pong` the Server's; the rest go both ways.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StreamMessage {
    Open {
        stream: u64,
        target: StreamTarget,
    },
    Opened {
        stream: u64,
        size: Option<u64>,
    },
    OpenFailed {
        stream: u64,
        error: StreamError,
        detail: String,
    },
    Data {
        stream: u64,
        data: Vec<u8>,
    },
    Credit {
        stream: u64,
        max_offset: u64,
    },
    Finish {
        stream: u64,
    },
    Reset {
        stream: u64,
        error: StreamError,
        detail: String,
    },
    Ping,
    Pong,
    /// The sender is closing the connection; the receiver closes its own at once, since
    /// some transports cannot interrupt a blocked read.
    Close,
    /// A top-level member from a newer peer; ignored.
    Unknown,
}

/// A frame the Client sends on a Stream connection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamClientMessage(pub StreamMessage);

/// A frame the Server sends on a Stream connection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamServerMessage(pub StreamMessage);
