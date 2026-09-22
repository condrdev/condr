//! Versioned, transport-independent server/client protocol.
//!
//! The server owns the runtime. A frozen [`ClientHandshake`] receives only a frozen
//! [`Welcome`] (see `handshake`). Clients explicitly request a lightweight overview
//! or a complete Bootstrap, then optionally subscribe to ordered reliable events
//! and coalesced terminal visual frames. The same framing works over local IPC
//! and authenticated, encrypted TCP transports.

mod bootstrap;
mod framing;
mod handshake;
mod messages;

use crate::{
    AgentSnapshot, PaneDirection, PaneId, SessionSnapshot, SplitDirection, TabId, TerminalCommand,
    TerminalView, TerminalViewFrame, WorkspaceId,
};
use bincode::Options as _;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::io::{self, Read, Write};
use std::path::PathBuf;

pub use relative_path::{RelativePath, RelativePathBuf};

pub use bootstrap::BootstrapAssembler;
pub use framing::{
    FramingError, decode_bootstrap_record, decode_pane_terminal_frame, encode_bootstrap_record,
    encode_pane_terminal_frame, read_message, read_message_with_limit, write_client_message,
    write_message, write_message_with_limit,
};
pub use handshake::{ClientHandshake, Hello, Refusal, Welcome};
pub use messages::{
    AgentCommand, AgentError, AgentInfo, AgentResponse, BootstrapBatch, BootstrapHeader,
    BootstrapRecord, ClientMessage, ClipboardImageFormat, DiffBase, LayoutCommand, LayoutResult,
    PaneAgentSnapshot, PaneTerminalFrame, PaneTerminalMetadata, PaneTerminalSnapshot, RuntimeEpoch,
    ServerAdminCommand, ServerAdminResponse, ServerClientInfo, ServerId, ServerLogRecord,
    ServerMessage, ServerSettings, SessionBootstrap, SessionEvent, SessionId, SessionOverview,
    TerminalFrameBatch, TerminalFrameChunk, WorkspaceGitSnapshot, relative_age, uptime_text,
};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_FRAME_SIZE: usize = 2 * 1024 * 1024;
/// The largest clipboard image a Client may paste into a remote Pane (ADR 0012).
pub const MAX_CLIPBOARD_IMAGE_BYTES: usize = 16 * 1024 * 1024;
/// The inbound frame allowance for `ClientMessage::PasteImage` alone: the image plus the
/// message's own fields. Every other message keeps `MAX_FRAME_SIZE`.
pub const MAX_IMAGE_FRAME_SIZE: usize = MAX_CLIPBOARD_IMAGE_BYTES + 1024;
pub const MAX_BOOTSTRAP_BATCHES: u32 = 65_536;
pub const MAX_CHUNKED_RECORD_SIZE: usize = 32 * 1024 * 1024;
pub const MAX_BOOTSTRAP_TOTAL_SIZE: usize = 64 * 1024 * 1024;
pub const MAX_CHUNK_PAYLOAD_SIZE: usize = MAX_FRAME_SIZE - 256;
/// Bytes a framed `BootstrapBatch` adds on top of its payload: the 4-byte length prefix plus
/// the bincode envelope (discriminant, identifiers, indices, payload length).
pub const BOOTSTRAP_BATCH_FRAME_OVERHEAD: usize = 64;
/// The largest complete framed Bootstrap the limits above allow: one header frame, every
/// batch envelope, and the aggregate record payload.
pub const MAX_FRAMED_BOOTSTRAP_BYTES: usize = (MAX_FRAME_SIZE + 4)
    + MAX_BOOTSTRAP_TOTAL_SIZE
    + MAX_BOOTSTRAP_BATCHES as usize * BOOTSTRAP_BATCH_FRAME_OVERHEAD;
