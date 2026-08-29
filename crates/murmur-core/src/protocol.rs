//! Versioned, transport-independent server/client protocol.
//!
//! The server owns the runtime. A client starts with [`ClientMessage::Hello`],
//! receives a [`ServerMessage::Bootstrap`], and then consumes ordered reliable
//! events plus coalesced terminal visual frames. The same framing works over
//! local IPC and a future TCP/SSH transport.

use std::collections::HashSet;
use std::fmt;
use std::io::{self, Read, Write};
use std::path::PathBuf;

use bincode::Options as _;
use serde::{Deserialize, Serialize};

use crate::{
    AgentSnapshot, PaneDirection, PaneId, SessionSnapshot, SplitDirection, TabId, TerminalCommand,
    TerminalView, TerminalViewFrame, WorkspaceId,
};

pub const PROTOCOL_VERSION: u32 = 1;
pub const MAX_FRAME_SIZE: usize = 2 * 1024 * 1024;
pub const MAX_BOOTSTRAP_BATCHES: u32 = 65_536;
pub const MAX_CHUNKED_RECORD_SIZE: usize = 32 * 1024 * 1024;
pub const MAX_BOOTSTRAP_TOTAL_SIZE: usize = 64 * 1024 * 1024;
pub const MAX_CHUNK_PAYLOAD_SIZE: usize = MAX_FRAME_SIZE - 256;

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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    Layout {
        server_id: ServerId,
        session_id: SessionId,
        command: LayoutCommand,
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
pub enum LayoutCommand {
    CreateWorkspace {
        root_directory: PathBuf,
    },
    CreateWorktree {
        parent_workspace_id: WorkspaceId,
        branch: String,
    },
    OpenWorktree {
        parent_workspace_id: WorkspaceId,
        root_directory: PathBuf,
    },
    RemoveWorktree {
        workspace_id: WorkspaceId,
    },
    CreateTab {
        workspace_id: WorkspaceId,
    },
    RenameWorkspace {
        workspace_id: WorkspaceId,
        name: String,
    },
    RenameTab {
        tab_id: TabId,
        name: String,
    },
    ActivateWorkspace {
        workspace_id: WorkspaceId,
    },
    ActivateTab {
        tab_id: TabId,
    },
    MoveWorkspace {
        workspace_id: WorkspaceId,
        target_index: u32,
    },
    MoveTab {
        tab_id: TabId,
        target_index: u32,
    },
    SplitPane {
        pane_id: PaneId,
        direction: SplitDirection,
    },
    FocusPane {
        pane_id: PaneId,
    },
    FocusPaneDirection {
        pane_id: PaneId,
        direction: PaneDirection,
    },
    ResizePane {
        pane_id: PaneId,
        direction: PaneDirection,
        amount: f32,
    },
    SetSplitRatios {
        tab_id: TabId,
        ratios: Vec<f32>,
    },
    SwapPane {
        pane_id: PaneId,
        direction: PaneDirection,
    },
    TogglePaneZoom {
        pane_id: PaneId,
    },
    ClosePane {
        pane_id: PaneId,
    },
    CloseTab {
        tab_id: TabId,
    },
    CloseWorkspace {
        workspace_id: WorkspaceId,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BootstrapHeader {
    pub server_id: ServerId,
    pub runtime_epoch: RuntimeEpoch,
    pub session_id: SessionId,
    pub sequence: u64,
    pub snapshot: SessionSnapshot,
    pub batch_count: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionBootstrap {
    pub server_id: ServerId,
    pub runtime_epoch: RuntimeEpoch,
    pub session_id: SessionId,
    pub sequence: u64,
    pub snapshot: SessionSnapshot,
    pub terminals: Vec<PaneTerminalSnapshot>,
    pub agents: Vec<PaneAgentSnapshot>,
    pub workspace_git: Vec<WorkspaceGitSnapshot>,
    pub zoomed_panes: Vec<PaneId>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BootstrapBatch {
    pub server_id: ServerId,
    pub session_id: SessionId,
    pub batch_index: u32,
    pub record_index: u32,
    pub chunk_index: u32,
    pub chunk_count: u32,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PaneTerminalSnapshot {
    pub pane_id: PaneId,
    pub view: TerminalView,
    pub exited: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PaneAgentSnapshot {
    pub pane_id: PaneId,
    pub agent: AgentSnapshot,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceGitSnapshot {
    pub workspace_id: WorkspaceId,
    pub branch: Option<String>,
    pub linked_worktree: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum BootstrapRecord {
    Terminal(PaneTerminalSnapshot),
    Agent(PaneAgentSnapshot),
    WorkspaceGit(WorkspaceGitSnapshot),
    ZoomedPane(PaneId),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PaneTerminalFrame {
    pub pane_id: PaneId,
    pub frame: TerminalViewFrame,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalFrameBatch {
    pub server_id: ServerId,
    pub session_id: SessionId,
    pub panes: Vec<PaneTerminalFrame>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TerminalFrameChunk {
    pub server_id: ServerId,
    pub session_id: SessionId,
    pub pane_id: PaneId,
    pub revision: u64,
    pub chunk_index: u32,
    pub chunk_count: u32,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SessionEvent {
    LayoutChanged,
    TerminalExited {
        pane_id: PaneId,
    },
    AgentChanged {
        pane_id: PaneId,
        agent: Option<AgentSnapshot>,
    },
    WorkspaceGitChanged {
        workspace_id: WorkspaceId,
        git: Option<WorkspaceGitSnapshot>,
    },
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
    Bootstrap(BootstrapHeader),
    BootstrapBatch(BootstrapBatch),
    Subscribed {
        server_id: ServerId,
        session_id: SessionId,
        sequence: u64,
    },
    SubscriptionRejected {
        server_id: ServerId,
        session_id: SessionId,
        reason: String,
    },
    Event {
        server_id: ServerId,
        session_id: SessionId,
        sequence: u64,
        event: SessionEvent,
    },
    TerminalFrame(TerminalFrameBatch),
    TerminalFrameChunk(TerminalFrameChunk),
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

pub fn encode_bootstrap_record(record: &BootstrapRecord) -> Result<Vec<u8>, String> {
    encode_chunked_value(record, "Bootstrap record")
}

pub fn decode_bootstrap_record(payload: &[u8]) -> Result<BootstrapRecord, String> {
    decode_chunked_value(payload, "Bootstrap record")
}

pub fn encode_pane_terminal_frame(frame: &PaneTerminalFrame) -> Result<Vec<u8>, String> {
    encode_chunked_value(frame, "Pane terminal frame")
}

pub fn decode_pane_terminal_frame(payload: &[u8]) -> Result<PaneTerminalFrame, String> {
    decode_chunked_value(payload, "Pane terminal frame")
}

fn encode_chunked_value<T>(value: &T, description: &str) -> Result<Vec<u8>, String>
where
    T: Serialize,
{
    let payload = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_CHUNKED_RECORD_SIZE as u64)
        .serialize(value)
        .map_err(|error| format!("{description} cannot be encoded: {error}"))?;
    if payload.len() > MAX_CHUNKED_RECORD_SIZE {
        return Err(format!(
            "{description} is {} bytes; limit is {MAX_CHUNKED_RECORD_SIZE} bytes",
            payload.len()
        ));
    }
    Ok(payload)
}

fn decode_chunked_value<T>(payload: &[u8], description: &str) -> Result<T, String>
where
    T: for<'de> Deserialize<'de>,
{
    if payload.len() > MAX_CHUNKED_RECORD_SIZE {
        return Err(format!(
            "{description} is {} bytes; limit is {MAX_CHUNKED_RECORD_SIZE} bytes",
            payload.len()
        ));
    }
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_CHUNKED_RECORD_SIZE as u64)
        .reject_trailing_bytes()
        .deserialize(payload)
        .map_err(|error| format!("{description} cannot be decoded: {error}"))
}

#[derive(Debug)]
pub struct BootstrapAssembler {
    header: BootstrapHeader,
    next_batch_index: u32,
    next_record_index: u32,
    next_chunk_index: u32,
    current_chunk_count: Option<u32>,
    current_payload: Vec<u8>,
    total_payload_size: usize,
    terminals: Vec<PaneTerminalSnapshot>,
    agents: Vec<PaneAgentSnapshot>,
    workspace_git: Vec<WorkspaceGitSnapshot>,
    zoomed_panes: Vec<PaneId>,
    terminal_ids: HashSet<PaneId>,
    agent_ids: HashSet<PaneId>,
    workspace_git_ids: HashSet<WorkspaceId>,
    zoomed_pane_ids: HashSet<PaneId>,
}

fn checked_bootstrap_total_size(current: usize, additional: usize) -> Result<usize, String> {
    let total = current
        .checked_add(additional)
        .ok_or_else(|| "Bootstrap total size overflowed usize".to_string())?;
    if total > MAX_BOOTSTRAP_TOTAL_SIZE {
        return Err(format!(
            "Bootstrap exceeds the {MAX_BOOTSTRAP_TOTAL_SIZE}-byte aggregate limit"
        ));
    }
    Ok(total)
}

impl BootstrapAssembler {
    pub fn new(header: BootstrapHeader) -> Result<Self, String> {
        if header.batch_count > MAX_BOOTSTRAP_BATCHES {
            return Err(format!(
                "Bootstrap declares {} batches; limit is {MAX_BOOTSTRAP_BATCHES}",
                header.batch_count
            ));
        }

        Ok(Self {
            header,
            next_batch_index: 0,
            next_record_index: 0,
            next_chunk_index: 0,
            current_chunk_count: None,
            current_payload: Vec::new(),
            total_payload_size: 0,
            terminals: Vec::new(),
            agents: Vec::new(),
            workspace_git: Vec::new(),
            zoomed_panes: Vec::new(),
            terminal_ids: HashSet::new(),
            agent_ids: HashSet::new(),
            workspace_git_ids: HashSet::new(),
            zoomed_pane_ids: HashSet::new(),
        })
    }

    pub fn push(&mut self, batch: BootstrapBatch) -> Result<(), String> {
        if self.next_batch_index >= self.header.batch_count {
            return Err("Bootstrap contains more batches than declared".into());
        }
        if batch.server_id != self.header.server_id || batch.session_id != self.header.session_id {
            return Err("Bootstrap batch identity does not match its header".into());
        }
        if batch.batch_index != self.next_batch_index {
            return Err(format!(
                "Bootstrap batch index {} is out of order; expected {}",
                batch.batch_index, self.next_batch_index
            ));
        }
        if batch.record_index != self.next_record_index {
            return Err(format!(
                "Bootstrap record index {} is out of order; expected {}",
                batch.record_index, self.next_record_index
            ));
        }
        if batch.chunk_count == 0 || batch.chunk_index >= batch.chunk_count {
            return Err("Bootstrap batch has an invalid chunk range".into());
        }
        if batch.chunk_index != self.next_chunk_index {
            return Err(format!(
                "Bootstrap chunk index {} is out of order; expected {}",
                batch.chunk_index, self.next_chunk_index
            ));
        }
        if batch.payload.is_empty() {
            return Err("Bootstrap chunk payload is empty".into());
        }
        if batch.payload.len() > MAX_CHUNK_PAYLOAD_SIZE {
            return Err(format!(
                "Bootstrap chunk is {} bytes; limit is {MAX_CHUNK_PAYLOAD_SIZE} bytes",
                batch.payload.len()
            ));
        }

        match self.current_chunk_count {
            Some(chunk_count) if chunk_count != batch.chunk_count => {
                return Err("Bootstrap chunk count changed within a record".into());
            }
            Some(_) => {}
            None => self.current_chunk_count = Some(batch.chunk_count),
        }
        let remaining_chunks = batch.chunk_count - batch.chunk_index;
        let remaining_batches = self.header.batch_count - self.next_batch_index;
        if remaining_chunks > remaining_batches {
            return Err("Bootstrap record cannot fit in the declared batch count".into());
        }

        let next_size = self
            .current_payload
            .len()
            .checked_add(batch.payload.len())
            .ok_or_else(|| "Bootstrap record size overflowed usize".to_string())?;
        if next_size > MAX_CHUNKED_RECORD_SIZE {
            return Err(format!(
                "Bootstrap record exceeds the {MAX_CHUNKED_RECORD_SIZE}-byte limit"
            ));
        }
        self.total_payload_size =
            checked_bootstrap_total_size(self.total_payload_size, batch.payload.len())?;
        self.current_payload
            .try_reserve(batch.payload.len())
            .map_err(|_| "Bootstrap record allocation failed".to_string())?;
        self.current_payload.extend_from_slice(&batch.payload);
        self.next_batch_index = self
            .next_batch_index
            .checked_add(1)
            .ok_or_else(|| "Bootstrap batch index overflowed u32".to_string())?;
        self.next_chunk_index = self
            .next_chunk_index
            .checked_add(1)
            .ok_or_else(|| "Bootstrap chunk index overflowed u32".to_string())?;

        if self.next_chunk_index == batch.chunk_count {
            let record = decode_bootstrap_record(&self.current_payload)?;
            self.insert_record(record)?;
            self.current_payload.clear();
            self.current_chunk_count = None;
            self.next_chunk_index = 0;
            self.next_record_index = self
                .next_record_index
                .checked_add(1)
                .ok_or_else(|| "Bootstrap record index overflowed u32".to_string())?;
        }
        Ok(())
    }

    pub fn finish(self) -> Result<SessionBootstrap, String> {
        if self.next_batch_index != self.header.batch_count {
            return Err(format!(
                "Bootstrap ended after {} of {} declared batches",
                self.next_batch_index, self.header.batch_count
            ));
        }
        if self.current_chunk_count.is_some() || !self.current_payload.is_empty() {
            return Err("Bootstrap ended in the middle of a record".into());
        }
        Ok(SessionBootstrap {
            server_id: self.header.server_id,
            runtime_epoch: self.header.runtime_epoch,
            session_id: self.header.session_id,
            sequence: self.header.sequence,
            snapshot: self.header.snapshot,
            terminals: self.terminals,
            agents: self.agents,
            workspace_git: self.workspace_git,
            zoomed_panes: self.zoomed_panes,
        })
    }

    fn insert_record(&mut self, record: BootstrapRecord) -> Result<(), String> {
        match record {
            BootstrapRecord::Terminal(terminal) => {
                if !self.terminal_ids.insert(terminal.pane_id) {
                    return Err("Bootstrap contains a duplicate Terminal Pane ID".into());
                }
                self.terminals.push(terminal);
            }
            BootstrapRecord::Agent(agent) => {
                if !self.agent_ids.insert(agent.pane_id) {
                    return Err("Bootstrap contains a duplicate Agent Pane ID".into());
                }
                self.agents.push(agent);
            }
            BootstrapRecord::WorkspaceGit(git) => {
                if !self.workspace_git_ids.insert(git.workspace_id) {
                    return Err("Bootstrap contains a duplicate Workspace Git ID".into());
                }
                self.workspace_git.push(git);
            }
            BootstrapRecord::ZoomedPane(pane_id) => {
                if !self.zoomed_pane_ids.insert(pane_id) {
                    return Err("Bootstrap contains a duplicate zoomed Pane ID".into());
                }
                self.zoomed_panes.push(pane_id);
            }
        }
        Ok(())
    }
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
    let payload = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_FRAME_SIZE as u64)
        .serialize(message)
        .map_err(|error| FramingError::Codec(error.to_string()))?;
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
    let message = bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_FRAME_SIZE as u64)
        .reject_trailing_bytes()
        .deserialize(&payload)
        .map_err(|error| FramingError::Codec(error.to_string()))?;
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
    fn layout_command_round_trip_uses_the_existing_protocol_version() {
        let message = ClientMessage::Layout {
            server_id: ServerId(4),
            session_id: SessionId(1),
            command: LayoutCommand::CreateWorkspace {
                root_directory: PathBuf::from("projects/murmur"),
            },
        };
        let mut bytes = Vec::new();
        write_message(&mut bytes, &message).unwrap();
        assert_eq!(
            read_message::<_, ClientMessage>(&mut bytes.as_slice()).unwrap(),
            message
        );
        assert_eq!(PROTOCOL_VERSION, 1);
    }

    #[test]
    fn worktree_command_round_trip_keeps_protocol_version_one() {
        let mut session = crate::Session::new();
        let parent_workspace_id = session
            .create_workspace(PathBuf::from("projects/murmur"))
            .expect("Workspace capacity");
        let message = ClientMessage::Layout {
            server_id: ServerId(4),
            session_id: SessionId(1),
            command: LayoutCommand::CreateWorktree {
                parent_workspace_id,
                branch: "feature/phase-five".into(),
            },
        };
        let mut bytes = Vec::new();
        write_message(&mut bytes, &message).unwrap();
        assert_eq!(
            read_message::<_, ClientMessage>(&mut bytes.as_slice()).unwrap(),
            message
        );
        assert_eq!(PROTOCOL_VERSION, 1);
    }

    #[test]
    fn subscription_rejection_round_trip_keeps_protocol_version_one() {
        let message = ServerMessage::SubscriptionRejected {
            server_id: ServerId(4),
            session_id: SessionId(7),
            reason: "event cursor expired".into(),
        };
        let mut bytes = Vec::new();
        write_message(&mut bytes, &message).unwrap();
        assert_eq!(
            read_message::<_, ServerMessage>(&mut bytes.as_slice()).unwrap(),
            message
        );
        assert_eq!(PROTOCOL_VERSION, 1);
    }

    #[test]
    fn bootstrap_header_round_trip_uses_the_flat_snapshot_schema() {
        let mut session = crate::Session::new();
        session
            .create_workspace(PathBuf::from("projects/murmur"))
            .expect("Workspace capacity");
        let pane_id = session
            .active_workspace()
            .expect("Workspace is active")
            .active_tab()
            .focused_pane()
            .id();
        session
            .split_pane(pane_id, SplitDirection::Horizontal, 0.5)
            .expect("Pane exists");
        let message = ServerMessage::Bootstrap(BootstrapHeader {
            server_id: ServerId(4),
            runtime_epoch: RuntimeEpoch(5),
            session_id: SessionId(1),
            sequence: 0,
            snapshot: session.snapshot(),
            batch_count: 0,
        });

        let mut bytes = Vec::new();
        write_message(&mut bytes, &message).unwrap();
        assert_eq!(
            read_message::<_, ServerMessage>(&mut bytes.as_slice()).unwrap(),
            message
        );
        assert_eq!(PROTOCOL_VERSION, 1);
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

    #[test]
    fn forged_collection_length_is_rejected_without_allocating_it() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&0u32.to_le_bytes());
        payload.extend_from_slice(&PROTOCOL_VERSION.to_le_bytes());
        payload.extend_from_slice(&u64::MAX.to_le_bytes());
        let mut frame = Vec::from((payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(&payload);

        assert!(matches!(
            read_message::<_, ClientMessage>(&mut frame.as_slice()),
            Err(FramingError::Codec(_))
        ));
    }

    #[test]
    fn bootstrap_assembler_reassembles_multiple_record_chunks() {
        let (mut header, pane_id, workspace_id) = bootstrap_header(0);
        let records = vec![
            BootstrapRecord::Terminal(terminal_snapshot(
                pane_id,
                "x".repeat(MAX_CHUNK_PAYLOAD_SIZE + 1_024),
            )),
            BootstrapRecord::Agent(PaneAgentSnapshot {
                pane_id,
                agent: AgentSnapshot {
                    kind: crate::AgentKind::Codex,
                    state: crate::AgentState::Working,
                },
            }),
            BootstrapRecord::WorkspaceGit(WorkspaceGitSnapshot {
                workspace_id,
                branch: Some("feature/wire-chunks".into()),
                linked_worktree: true,
            }),
            BootstrapRecord::ZoomedPane(pane_id),
        ];
        let batches = bootstrap_batches(header.server_id, header.session_id, &records);
        assert!(batches.len() > records.len());
        header.batch_count = batches.len() as u32;

        let mut assembler = BootstrapAssembler::new(header).unwrap();
        for batch in batches {
            assembler.push(batch).unwrap();
        }
        let assembled = assembler.finish().unwrap();

        assert_eq!(
            assembled.terminals,
            vec![match records[0].clone() {
                BootstrapRecord::Terminal(terminal) => terminal,
                _ => unreachable!(),
            }]
        );
        assert_eq!(
            assembled.agents,
            vec![match records[1].clone() {
                BootstrapRecord::Agent(agent) => agent,
                _ => unreachable!(),
            }]
        );
        assert_eq!(
            assembled.workspace_git,
            vec![match records[2].clone() {
                BootstrapRecord::WorkspaceGit(git) => git,
                _ => unreachable!(),
            }]
        );
        assert_eq!(assembled.zoomed_panes, vec![pane_id]);
    }

    #[test]
    fn bootstrap_assembler_rejects_order_identity_and_chunk_errors() {
        let (header, pane_id, _) = bootstrap_header(1);
        let payload = encode_bootstrap_record(&BootstrapRecord::ZoomedPane(pane_id)).unwrap();

        let mut assembler = BootstrapAssembler::new(header.clone()).unwrap();
        let mut out_of_order = bootstrap_batch(&header, 1, 0, 0, 1, payload.clone());
        assert!(assembler.push(out_of_order.clone()).is_err());

        out_of_order.batch_index = 0;
        out_of_order.server_id = ServerId(header.server_id.0 + 1);
        let mut assembler = BootstrapAssembler::new(header.clone()).unwrap();
        assert!(assembler.push(out_of_order).is_err());

        let mut assembler = BootstrapAssembler::new(header.clone()).unwrap();
        assert!(
            assembler
                .push(bootstrap_batch(&header, 0, 0, 1, 2, payload.clone()))
                .is_err()
        );

        let mut assembler = BootstrapAssembler::new(header).unwrap();
        assert!(
            assembler
                .push(BootstrapBatch {
                    server_id: ServerId(4),
                    session_id: SessionId(1),
                    batch_index: 0,
                    record_index: 0,
                    chunk_index: 0,
                    chunk_count: 1,
                    payload: vec![0; MAX_CHUNK_PAYLOAD_SIZE + 1],
                })
                .is_err()
        );
    }

    #[test]
    fn maximum_chunk_payloads_fit_the_outer_protocol_frame() {
        let (header, pane_id, _) = bootstrap_header(1);
        let messages = [
            ServerMessage::BootstrapBatch(bootstrap_batch(
                &header,
                0,
                0,
                0,
                1,
                vec![0; MAX_CHUNK_PAYLOAD_SIZE],
            )),
            ServerMessage::TerminalFrameChunk(TerminalFrameChunk {
                server_id: header.server_id,
                session_id: header.session_id,
                pane_id,
                revision: 1,
                chunk_index: 0,
                chunk_count: 1,
                payload: vec![0; MAX_CHUNK_PAYLOAD_SIZE],
            }),
        ];

        for message in messages {
            let mut frame = Vec::new();
            write_message(&mut frame, &message).unwrap();
            assert!(frame.len() <= MAX_FRAME_SIZE + 4);
        }
    }

    #[test]
    fn bootstrap_assembler_rejects_oversized_records_before_decode() {
        let (header, _, _) = bootstrap_header(17);
        let mut assembler = BootstrapAssembler::new(header.clone()).unwrap();
        let full_chunks = MAX_CHUNKED_RECORD_SIZE / MAX_CHUNK_PAYLOAD_SIZE;
        let tail = MAX_CHUNKED_RECORD_SIZE % MAX_CHUNK_PAYLOAD_SIZE;
        assert_eq!(full_chunks, 16);
        for chunk_index in 0..full_chunks {
            assembler
                .push(bootstrap_batch(
                    &header,
                    chunk_index as u32,
                    0,
                    chunk_index as u32,
                    17,
                    vec![0; MAX_CHUNK_PAYLOAD_SIZE],
                ))
                .unwrap();
        }
        assert!(
            assembler
                .push(bootstrap_batch(
                    &header,
                    full_chunks as u32,
                    0,
                    full_chunks as u32,
                    17,
                    vec![0; tail + 1],
                ))
                .is_err()
        );
    }

    #[test]
    fn chunked_record_codecs_reject_trailing_and_oversized_payloads() {
        let (_, pane_id, _) = bootstrap_header(0);
        let record = BootstrapRecord::ZoomedPane(pane_id);
        let mut encoded = encode_bootstrap_record(&record).unwrap();
        assert_eq!(decode_bootstrap_record(&encoded).unwrap(), record);
        encoded.push(0);
        assert!(decode_bootstrap_record(&encoded).is_err());

        let frame = PaneTerminalFrame {
            pane_id,
            frame: TerminalViewFrame::Full(terminal_snapshot(pane_id, "frame".into()).view),
        };
        let encoded = encode_pane_terminal_frame(&frame).unwrap();
        assert_eq!(decode_pane_terminal_frame(&encoded).unwrap(), frame);
        assert!(decode_pane_terminal_frame(&vec![0; MAX_CHUNKED_RECORD_SIZE + 1]).is_err());
    }

    #[test]
    fn bootstrap_assembler_rejects_duplicate_ids_per_record_kind() {
        let (header, pane_id, workspace_id) = bootstrap_header(2);
        let duplicate_records = [
            BootstrapRecord::Terminal(terminal_snapshot(pane_id, "terminal".into())),
            BootstrapRecord::Agent(PaneAgentSnapshot {
                pane_id,
                agent: AgentSnapshot {
                    kind: crate::AgentKind::Claude,
                    state: crate::AgentState::Idle,
                },
            }),
            BootstrapRecord::WorkspaceGit(WorkspaceGitSnapshot {
                workspace_id,
                branch: None,
                linked_worktree: false,
            }),
            BootstrapRecord::ZoomedPane(pane_id),
        ];

        for record in duplicate_records {
            let payload = encode_bootstrap_record(&record).unwrap();
            let mut assembler = BootstrapAssembler::new(header.clone()).unwrap();
            assembler
                .push(bootstrap_batch(&header, 0, 0, 0, 1, payload.clone()))
                .unwrap();
            assert!(
                assembler
                    .push(bootstrap_batch(&header, 1, 1, 0, 1, payload))
                    .is_err()
            );
        }
    }

    #[test]
    fn bootstrap_assembler_finishes_an_empty_bootstrap() {
        let (header, _, _) = bootstrap_header(0);
        let expected = SessionBootstrap {
            server_id: header.server_id,
            runtime_epoch: header.runtime_epoch,
            session_id: header.session_id,
            sequence: header.sequence,
            snapshot: header.snapshot.clone(),
            terminals: Vec::new(),
            agents: Vec::new(),
            workspace_git: Vec::new(),
            zoomed_panes: Vec::new(),
        };
        assert_eq!(
            BootstrapAssembler::new(header).unwrap().finish().unwrap(),
            expected
        );
    }

    #[test]
    fn bootstrap_aggregate_limit_rejects_overflow_without_allocating_the_payload() {
        assert_eq!(
            checked_bootstrap_total_size(MAX_BOOTSTRAP_TOTAL_SIZE - 1, 1).unwrap(),
            MAX_BOOTSTRAP_TOTAL_SIZE
        );
        assert!(checked_bootstrap_total_size(MAX_BOOTSTRAP_TOTAL_SIZE, 1).is_err());
        assert!(checked_bootstrap_total_size(usize::MAX, 1).is_err());
    }

    fn bootstrap_header(batch_count: u32) -> (BootstrapHeader, PaneId, WorkspaceId) {
        let mut session = crate::Session::new();
        let workspace_id = session
            .create_workspace(PathBuf::from("projects/murmur"))
            .expect("Workspace capacity");
        let pane_id = session
            .active_workspace()
            .unwrap()
            .active_tab()
            .focused_pane()
            .id();
        (
            BootstrapHeader {
                server_id: ServerId(4),
                runtime_epoch: RuntimeEpoch(5),
                session_id: SessionId(1),
                sequence: 7,
                snapshot: session.snapshot(),
                batch_count,
            },
            pane_id,
            workspace_id,
        )
    }

    fn terminal_snapshot(pane_id: PaneId, text: String) -> PaneTerminalSnapshot {
        PaneTerminalSnapshot {
            pane_id,
            view: TerminalView {
                revision: 11,
                size: crate::TerminalSize::new(1, 1),
                display_offset: 0,
                cells: vec![crate::TerminalCell {
                    text: text.into(),
                    foreground: crate::TerminalColor::Named(0),
                    background: crate::TerminalColor::Named(0),
                    flags: 0,
                }],
                cursor: None,
            },
            exited: false,
        }
    }

    fn bootstrap_batches(
        server_id: ServerId,
        session_id: SessionId,
        records: &[BootstrapRecord],
    ) -> Vec<BootstrapBatch> {
        let mut batches = Vec::new();
        for (record_index, record) in records.iter().enumerate() {
            let payload = encode_bootstrap_record(record).unwrap();
            let chunk_count = payload.len().div_ceil(MAX_CHUNK_PAYLOAD_SIZE) as u32;
            for (chunk_index, payload) in payload.chunks(MAX_CHUNK_PAYLOAD_SIZE).enumerate() {
                batches.push(BootstrapBatch {
                    server_id,
                    session_id,
                    batch_index: batches.len() as u32,
                    record_index: record_index as u32,
                    chunk_index: chunk_index as u32,
                    chunk_count,
                    payload: payload.to_vec(),
                });
            }
        }
        batches
    }

    fn bootstrap_batch(
        header: &BootstrapHeader,
        batch_index: u32,
        record_index: u32,
        chunk_index: u32,
        chunk_count: u32,
        payload: Vec<u8>,
    ) -> BootstrapBatch {
        BootstrapBatch {
            server_id: header.server_id,
            session_id: header.session_id,
            batch_index,
            record_index,
            chunk_index,
            chunk_count,
            payload,
        }
    }
}
