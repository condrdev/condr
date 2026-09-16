use super::*;

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
    /// Structural and lifecycle metadata, without capturing terminal views or subscribing.
    OverviewRequest {
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
        request_id: u64,
        command: LayoutCommand,
    },
    Terminal {
        server_id: ServerId,
        session_id: SessionId,
        pane_id: PaneId,
        command: TerminalCommand,
    },
    /// A clipboard image from the Client's machine for an Agent in a remote Pane (ADR
    /// 0012). The Server stages it in a private file and pastes that path into the Pane;
    /// nothing of it enters Session state. Same authority as `Terminal` text: no Session
    /// control needed. The only message allowed `MAX_IMAGE_FRAME_SIZE`.
    PasteImage {
        server_id: ServerId,
        session_id: SessionId,
        pane_id: PaneId,
        format: ClipboardImageFormat,
        bytes: Vec<u8>,
    },
    /// The last `lines` rows of a Pane as plain text, scrollback included: how the CLI
    /// and agents read a terminal. No Session control needed.
    ReadPane {
        server_id: ServerId,
        session_id: SessionId,
        pane_id: PaneId,
        lines: u32,
    },
    /// One file's working-tree diff against `HEAD` (ADR 0017), answered with
    /// [`ServerMessage::GitDiff`]. No Session control needed. `against` is fixed to `HEAD`
    /// today and reserved for the branch-against-base view.
    GitDiff {
        server_id: ServerId,
        session_id: SessionId,
        request_id: u64,
        workspace_id: WorkspaceId,
        /// Relative to the Workspace root.
        path: PathBuf,
        against: DiffBase,
    },
    /// One level of a Workspace's directory tree for the Files sidebar (ADR 0018),
    /// answered with [`ServerMessage::Directory`]. No Session control needed.
    ListDirectory {
        server_id: ServerId,
        session_id: SessionId,
        request_id: u64,
        workspace_id: WorkspaceId,
        /// Relative to the Workspace root; empty for the root itself.
        path: PathBuf,
    },
    /// One file's content for the Preview Tab (ADR 0018), answered with
    /// [`ServerMessage::FileContent`]. No Session control needed.
    ReadFile {
        server_id: ServerId,
        session_id: SessionId,
        request_id: u64,
        workspace_id: WorkspaceId,
        /// Relative to the Workspace root.
        path: PathBuf,
    },
    /// Agent orchestration is owned by the Server, including name resolution and waits.
    Agent {
        server_id: ServerId,
        session_id: SessionId,
        command: AgentCommand,
    },
    /// Replaces the Server's shell preference; blank restores the system default.
    /// Any client may do this, no Session control needed.
    SetServerSettings {
        server_id: ServerId,
        shell: String,
    },
    StopServer {
        server_id: ServerId,
    },
    /// Drops the live TCP connections of the device identified by its full public key.
    /// Only the Server host may send it; the authorized list itself is
    /// a file the host edits directly.
    RevokeDevice {
        key: String,
    },
    /// Asks which paired devices hold a live TCP connection right now. Server host only.
    ConnectedDevices,
    /// Server management commands. Only local/SSH connections may administer.
    ServerAdmin {
        server_id: ServerId,
        command: ServerAdminCommand,
    },
    Detach,
}

/// What a diff compares the working tree against.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum DiffBase {
    #[default]
    Head,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ServerAdminCommand {
    Status,
    SaveListen { address: Option<String> },
    Restart,
    Clients,
    Invite,
    Revoke { key: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServerClientInfo {
    pub name: String,
    pub fingerprint: String,
    pub last_seen: u64,
}

/// How long ago a unix timestamp was, as the CLI and GUI both print it.
pub fn relative_age(unix_seconds: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let elapsed = now.saturating_sub(unix_seconds);
    let (amount, unit) = match elapsed {
        0..60 => return "just now".into(),
        60..3_600 => (elapsed / 60, "minute"),
        3_600..86_400 => (elapsed / 3_600, "hour"),
        _ => (elapsed / 86_400, "day"),
    };
    format!("{amount} {unit}{} ago", if amount == 1 { "" } else { "s" })
}

#[cfg(test)]
mod relative_age_tests {
    use super::relative_age;

    #[test]
    fn reads_as_a_coarse_relative_time() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert_eq!(relative_age(now), "just now");
        assert_eq!(relative_age(now - 60), "1 minute ago");
        assert_eq!(relative_age(now - 5 * 60), "5 minutes ago");
        assert_eq!(relative_age(now - 3 * 3_600), "3 hours ago");
        assert_eq!(relative_age(now - 86_400), "1 day ago");
        assert_eq!(relative_age(now - 40 * 86_400), "40 days ago");
        assert_eq!(
            relative_age(now + 100),
            "just now",
            "a clock skew is not the future"
        );
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AgentCommand {
    Available,
    List,
    Start {
        name: String,
        kind: crate::AgentKind,
        pane_id: PaneId,
        args: Vec<String>,
        timeout_ms: u64,
    },
    Prompt {
        target: String,
        text: String,
        until: Option<Vec<crate::AgentState>>,
        timeout_ms: u64,
    },
    Wait {
        target: String,
        until: Vec<crate::AgentState>,
        timeout_ms: u64,
    },
    /// Installs, removes or inspects an agent's status hooks on the Server's machine,
    /// where the agents run and their configuration lives.
    Hooks {
        agent: crate::AgentKind,
        action: crate::agent_hooks::HooksAction,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentInfo {
    pub pane_id: PaneId,
    pub name: Option<String>,
    pub agent: AgentSnapshot,
    pub launch_pending: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AgentResponse {
    Available(Vec<crate::agent_discovery::AgentInstallation>),
    List(Vec<AgentInfo>),
    Ready(AgentInfo),
    Hooks(crate::agent_hooks::HooksReport),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentError {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum LayoutCommand {
    CreateWorkspace {
        root_directory: PathBuf,
        name: Option<String>,
        focus: bool,
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
        name: Option<String>,
        focus: bool,
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
        focus: bool,
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
    /// Detach a Pane and reattach it beside `target_pane_id` on `side`, in the same Tab.
    MovePane {
        pane_id: PaneId,
        target_pane_id: PaneId,
        side: PaneDirection,
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
    /// Shows `path`'s working-tree diff in the Workspace's single Diff Tab, creating the
    /// Tab the first time and retargeting it afterwards, and activates it (ADR 0017).
    ShowDiff {
        workspace_id: WorkspaceId,
        /// Relative to the Workspace root.
        path: PathBuf,
    },
    /// Shows `path`'s content in the Workspace's single Preview Tab the same way
    /// (ADR 0018).
    ShowFile {
        workspace_id: WorkspaceId,
        /// Relative to the Workspace root.
        path: PathBuf,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub enum LayoutResult {
    #[default]
    Changed,
    WorkspaceCreated {
        workspace_id: WorkspaceId,
        tab_id: TabId,
        pane_id: PaneId,
    },
    TabCreated {
        tab_id: TabId,
        pane_id: PaneId,
    },
    PaneCreated {
        pane_id: PaneId,
    },
    DiffShown {
        tab_id: TabId,
    },
    FileShown {
        tab_id: TabId,
    },
}

/// A lightweight authoritative Session query for CLI inspection and orchestration.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionOverview {
    pub server_id: ServerId,
    pub runtime_epoch: RuntimeEpoch,
    pub session_id: SessionId,
    pub sequence: u64,
    pub snapshot: SessionSnapshot,
    pub terminals: Vec<PaneTerminalMetadata>,
    pub agents: Vec<PaneAgentSnapshot>,
    pub zoomed_panes: Vec<PaneId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PaneTerminalMetadata {
    pub pane_id: PaneId,
    pub title: Option<String>,
    pub exited: bool,
}

impl From<&SessionBootstrap> for SessionOverview {
    fn from(bootstrap: &SessionBootstrap) -> Self {
        Self {
            server_id: bootstrap.server_id,
            runtime_epoch: bootstrap.runtime_epoch,
            session_id: bootstrap.session_id,
            sequence: bootstrap.sequence,
            snapshot: bootstrap.snapshot.clone(),
            terminals: bootstrap
                .terminals
                .iter()
                .map(|terminal| PaneTerminalMetadata {
                    pane_id: terminal.pane_id,
                    title: terminal.title.clone(),
                    exited: terminal.exited,
                })
                .collect(),
            agents: bootstrap.agents.clone(),
            zoomed_panes: bootstrap.zoomed_panes.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BootstrapHeader {
    pub server_id: ServerId,
    pub runtime_epoch: RuntimeEpoch,
    pub session_id: SessionId,
    pub sequence: u64,
    pub snapshot: SessionSnapshot,
    pub settings: ServerSettings,
    pub batch_count: u32,
}

/// Server-owned preferences, persisted in the Server's own `config.toml`. Clients
/// change them through [`ClientMessage::SetServerSettings`] and learn the current
/// values from the Bootstrap and [`SessionEvent::ServerSettingsChanged`].
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct ServerSettings {
    /// Program started in new terminals; empty means `default_shell`.
    pub shell: String,
    /// The system default shell this Server resolved, for display only.
    pub default_shell: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SessionBootstrap {
    pub server_id: ServerId,
    pub runtime_epoch: RuntimeEpoch,
    pub session_id: SessionId,
    pub sequence: u64,
    pub snapshot: SessionSnapshot,
    pub settings: ServerSettings,
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
    /// The OSC 0/2 title the Terminal last reported, already sanitized by the Server.
    pub title: Option<String>,
    /// Whether the active controller still needs to acknowledge a BEL from this Pane.
    pub attention: bool,
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
    pub upstream: Option<crate::GitUpstream>,
    /// The working tree against `HEAD` (ADR 0017), computed in the same pass as the rest.
    pub changes: crate::GitChanges,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum SessionEvent {
    /// The Session structure after a layout change. It carries the new structural
    /// Snapshot (a few KB: ids, names, split trees) and the live zoom state, so a client
    /// applies it in place instead of requesting a full Bootstrap and going dark until
    /// that arrives.
    LayoutChanged {
        snapshot: SessionSnapshot,
        zoomed_panes: Vec<PaneId>,
    },
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
    /// Something under the Workspace root changed on disk (ADR 0018): one debounced
    /// batch of watcher events, repository or not. Which paths is not said; a client
    /// asks again for the listings and files it holds. Batches that touch only ignored
    /// paths are not reported.
    WorkspaceFilesChanged {
        workspace_id: WorkspaceId,
    },
    TerminalTitleChanged {
        pane_id: PaneId,
        title: Option<String>,
    },
    /// The active controller's ordered, coalesced BEL attention state for this Pane.
    TerminalAttentionChanged {
        pane_id: PaneId,
        attention: bool,
    },
    ServerSettingsChanged {
        settings: ServerSettings,
    },
}

/// The image encodings a pasted clipboard image may arrive in. The Server names the
/// staged file after the format; the Client never supplies a name.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ClipboardImageFormat {
    Png,
    Jpeg,
    Gif,
    Webp,
    Bmp,
}

impl ClipboardImageFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Gif => "gif",
            Self::Webp => "webp",
            Self::Bmp => "bmp",
        }
    }
}

impl ClientMessage {
    /// How large this message's frame may be: images get their own allowance.
    pub fn frame_limit(&self) -> usize {
        match self {
            Self::PasteImage { .. } => MAX_IMAGE_FRAME_SIZE,
            _ => MAX_FRAME_SIZE,
        }
    }
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
    /// Overview header with an empty terminal list; exactly one OverviewTerminals follows.
    Overview(SessionOverview),
    OverviewTerminals {
        server_id: ServerId,
        session_id: SessionId,
        terminals: Vec<PaneTerminalMetadata>,
    },
    BootstrapBatch(BootstrapBatch),
    Subscribed {
        server_id: ServerId,
        session_id: SessionId,
        sequence: u64,
    },
    SnapshotRejected {
        server_id: ServerId,
        session_id: SessionId,
        reason: String,
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
    LayoutRejected {
        server_id: ServerId,
        session_id: SessionId,
        request_id: u64,
        reason: String,
    },
    LayoutApplied {
        server_id: ServerId,
        session_id: SessionId,
        request_id: u64,
        sequence: u64,
        result: LayoutResult,
    },
    TerminalCopied {
        pane_id: PaneId,
        text: Option<String>,
    },
    /// The reply to [`ClientMessage::ReadPane`]: rows joined by newline, trailing blank
    /// rows dropped.
    PaneText {
        pane_id: PaneId,
        text: String,
    },
    /// The reply to [`ClientMessage::GitDiff`].
    GitDiff {
        request_id: u64,
        workspace_id: WorkspaceId,
        /// Relative to the Workspace root, as requested.
        path: PathBuf,
        result: Result<crate::FileDiff, String>,
    },
    /// The reply to [`ClientMessage::ListDirectory`].
    Directory {
        request_id: u64,
        workspace_id: WorkspaceId,
        /// Relative to the Workspace root, as requested.
        path: PathBuf,
        result: Result<crate::DirectoryListing, String>,
    },
    /// The reply to [`ClientMessage::ReadFile`].
    FileContent {
        request_id: u64,
        workspace_id: WorkspaceId,
        /// Relative to the Workspace root, as requested.
        path: PathBuf,
        result: Result<crate::FileContent, String>,
    },
    AgentResult {
        result: Result<AgentResponse, AgentError>,
    },
    /// A program in the Pane copied text with OSC 52; every subscribed client receives it.
    TerminalClipboard {
        pane_id: PaneId,
        text: String,
    },
    ServerStopping,
    /// How many live connections a `RevokeDevice` request closed.
    DevicesRevoked {
        disconnected: u32,
    },
    /// The hex public keys of devices connected over TCP right now.
    ConnectedDevices {
        keys: Vec<String>,
    },
    ServerAdmin(ServerAdminResponse),
    Error {
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ServerAdminResponse {
    Status {
        listen: Option<String>,
        connected: Vec<String>,
    },
    ListenSaved {
        listen: Option<String>,
    },
    Clients {
        clients: Vec<ServerClientInfo>,
        connected: Vec<String>,
    },
    Invite {
        address: String,
        expires_in_secs: u64,
    },
    Revoked {
        disconnected: u32,
    },
}
