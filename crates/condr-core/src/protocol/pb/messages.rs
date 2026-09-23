//! `ClientMessage`, `ServerMessage` and what only they carry.

use super::session::{
    agent_state, command_agent_kind, hooks_action, pane_direction, split_direction,
};
use super::*;
use crate::agent_discovery::AgentInstallation;
use crate::agent_hooks::HooksReport;
use crate::protocol::{
    AgentCommand, AgentError, AgentInfo, AgentResponse, BootstrapBatch, BootstrapHeader,
    BootstrapRecord, ClientMessage, ClipboardImageFormat, DiffBase, LayoutCommand, LayoutResult,
    PaneAgentSnapshot, PaneTerminalFrame, PaneTerminalMetadata, PaneTerminalSnapshot, RuntimeEpoch,
    ServerAdminCommand, ServerAdminResponse, ServerClientInfo, ServerId, ServerLogRecord,
    ServerMessage, ServerSettings, SessionEvent, SessionId, SessionOverview, TerminalFrameBatch,
    TerminalFrameChunk, UnknownMessage, WorkspaceGitSnapshot,
};
use crate::{
    AgentKind, AgentSnapshot, DirectoryListing, FileContent, FileDiff, PaneId, SessionSnapshot,
    TabId, TerminalCommand, TerminalView, TerminalViewFrame, WorkspaceId,
};

fn pane(id: u64) -> PaneId {
    PaneId::from_u64(id)
}

fn tab(id: u64) -> TabId {
    TabId::from_u64(id)
}

fn workspace(id: u64) -> WorkspaceId {
    WorkspaceId::from_u64(id)
}

fn encode_kind(kind: AgentKind) -> i32 {
    super::AgentKind::from(kind) as i32
}

fn encode_epoch(epoch: RuntimeEpoch) -> super::RuntimeEpoch {
    super::RuntimeEpoch {
        value: epoch.0.to_be_bytes().to_vec(),
    }
}

fn decode_epoch(epoch: Option<super::RuntimeEpoch>) -> WireResult<RuntimeEpoch> {
    let bytes: [u8; 16] = required(epoch, "runtime epoch")?
        .value
        .try_into()
        .map_err(|_| malformed("runtime epoch is not 16 bytes"))?;
    Ok(RuntimeEpoch(u128::from_be_bytes(bytes)))
}

fn encode_snapshot(snapshot: &SessionSnapshot) -> Result<Option<super::SessionSnapshot>, String> {
    super::SessionSnapshot::try_from(snapshot).map(Some)
}

fn decode_snapshot(snapshot: Option<super::SessionSnapshot>) -> WireResult<SessionSnapshot> {
    required(snapshot, "Session Snapshot")?.try_into()
}

fn encode_agent(agent: &AgentSnapshot) -> Option<super::AgentSnapshot> {
    Some(agent.into())
}

fn decode_agent(agent: Option<super::AgentSnapshot>) -> WireResult<AgentSnapshot> {
    required(agent, "agent")?.try_into()
}

fn encode_settings(settings: &ServerSettings) -> super::ServerSettings {
    super::ServerSettings {
        shell: settings.shell.clone(),
        default_shell: settings.default_shell.clone(),
    }
}

fn decode_settings(settings: super::ServerSettings) -> ServerSettings {
    ServerSettings {
        shell: settings.shell,
        default_shell: settings.default_shell,
    }
}

fn encode_metadata(terminals: &[PaneTerminalMetadata]) -> Vec<super::PaneTerminalMetadata> {
    terminals
        .iter()
        .map(|terminal| super::PaneTerminalMetadata {
            pane_id: terminal.pane_id.as_u64(),
            title: terminal.title.clone(),
            exited: terminal.exited,
        })
        .collect()
}

fn decode_metadata(terminals: Vec<super::PaneTerminalMetadata>) -> Vec<PaneTerminalMetadata> {
    terminals
        .into_iter()
        .map(|terminal| PaneTerminalMetadata {
            pane_id: pane(terminal.pane_id),
            title: terminal.title,
            exited: terminal.exited,
        })
        .collect()
}

fn encode_pane_agent(agent: &PaneAgentSnapshot) -> super::PaneAgentSnapshot {
    super::PaneAgentSnapshot {
        pane_id: agent.pane_id.as_u64(),
        agent: encode_agent(&agent.agent),
    }
}

fn decode_pane_agent(agent: super::PaneAgentSnapshot) -> WireResult<PaneAgentSnapshot> {
    Ok(PaneAgentSnapshot {
        pane_id: pane(agent.pane_id),
        agent: decode_agent(agent.agent)?,
    })
}

fn encode_git(git: &WorkspaceGitSnapshot) -> super::WorkspaceGitSnapshot {
    super::WorkspaceGitSnapshot {
        workspace_id: git.workspace_id.as_u64(),
        branch: git.branch.clone(),
        linked_worktree: git.linked_worktree,
        upstream: git.upstream.as_ref().map(Into::into),
        changes: Some((&git.changes).into()),
    }
}

fn decode_git(git: super::WorkspaceGitSnapshot) -> WireResult<WorkspaceGitSnapshot> {
    Ok(WorkspaceGitSnapshot {
        workspace_id: workspace(git.workspace_id),
        branch: git.branch,
        linked_worktree: git.linked_worktree,
        upstream: git.upstream.map(Into::into),
        changes: required(git.changes, "Git changes")?.try_into()?,
    })
}

fn pane_ids(panes: &[PaneId]) -> Vec<u64> {
    panes.iter().map(|pane| pane.as_u64()).collect()
}

impl TryFrom<&ClientMessage> for super::ClientMessage {
    type Error = String;

    fn try_from(message: &ClientMessage) -> Result<Self, String> {
        use client_message::Message;
        let session = |session_id: &SessionId| SessionRequest {
            session_id: session_id.0,
        };
        let path_request = |server_id: &ServerId,
                            session_id: &SessionId,
                            request_id,
                            workspace_id: &WorkspaceId,
                            path: &RelativePathBuf| {
            PathRequest {
                server_id: server_id.0,
                session_id: session_id.0,
                request_id,
                workspace_id: workspace_id.as_u64(),
                path: path.to_string(),
            }
        };
        let message = match message {
            ClientMessage::SnapshotRequest { session_id } => {
                Message::SnapshotRequest(session(session_id))
            }
            ClientMessage::OverviewRequest { session_id } => {
                Message::OverviewRequest(session(session_id))
            }
            ClientMessage::Subscribe {
                session_id,
                after_sequence,
            } => Message::Subscribe(super::Subscribe {
                session_id: session_id.0,
                after_sequence: *after_sequence,
            }),
            ClientMessage::Ping { server_id, nonce } => Message::Ping(super::Ping {
                server_id: server_id.0,
                nonce: *nonce,
            }),
            ClientMessage::AcquireControl { session_id } => {
                Message::AcquireControl(session(session_id))
            }
            ClientMessage::ReleaseControl { session_id } => {
                Message::ReleaseControl(session(session_id))
            }
            ClientMessage::Layout {
                server_id,
                session_id,
                request_id,
                command,
            } => Message::Layout(LayoutRequest {
                server_id: server_id.0,
                session_id: session_id.0,
                request_id: *request_id,
                command: Some(command.try_into()?),
            }),
            ClientMessage::Terminal {
                server_id,
                session_id,
                pane_id,
                command,
            } => Message::Terminal(TerminalRequest {
                server_id: server_id.0,
                session_id: session_id.0,
                pane_id: pane_id.as_u64(),
                command: Some(command.into()),
            }),
            ClientMessage::PasteImage {
                server_id,
                session_id,
                pane_id,
                format,
                bytes,
            } => Message::PasteImage(super::PasteImage {
                server_id: server_id.0,
                session_id: session_id.0,
                pane_id: pane_id.as_u64(),
                format: match format {
                    ClipboardImageFormat::Png => super::ClipboardImageFormat::Png,
                    ClipboardImageFormat::Jpeg => super::ClipboardImageFormat::Jpeg,
                    ClipboardImageFormat::Gif => super::ClipboardImageFormat::Gif,
                    ClipboardImageFormat::Webp => super::ClipboardImageFormat::Webp,
                    ClipboardImageFormat::Bmp => super::ClipboardImageFormat::Bmp,
                } as i32,
                bytes: bytes.clone(),
            }),
            ClientMessage::ReadPane {
                server_id,
                session_id,
                pane_id,
                lines,
            } => Message::ReadPane(super::ReadPane {
                server_id: server_id.0,
                session_id: session_id.0,
                pane_id: pane_id.as_u64(),
                lines: *lines,
            }),
            ClientMessage::GitDiff {
                server_id,
                session_id,
                request_id,
                workspace_id,
                path,
                against: DiffBase::Head,
            } => Message::GitDiff(GitDiffRequest {
                server_id: server_id.0,
                session_id: session_id.0,
                request_id: *request_id,
                workspace_id: workspace_id.as_u64(),
                path: path.to_string(),
                against: super::DiffBase::Head as i32,
            }),
            ClientMessage::ListDirectory {
                server_id,
                session_id,
                request_id,
                workspace_id,
                path,
            } => Message::ListDirectory(path_request(
                server_id,
                session_id,
                *request_id,
                workspace_id,
                path,
            )),
            ClientMessage::ReadFile {
                server_id,
                session_id,
                request_id,
                workspace_id,
                path,
            } => Message::ReadFile(path_request(
                server_id,
                session_id,
                *request_id,
                workspace_id,
                path,
            )),
            ClientMessage::Agent {
                server_id,
                session_id,
                command,
            } => Message::Agent(AgentRequest {
                server_id: server_id.0,
                session_id: session_id.0,
                command: Some(command.into()),
            }),
            ClientMessage::SetServerSettings { server_id, shell } => {
                Message::SetServerSettings(super::SetServerSettings {
                    server_id: server_id.0,
                    shell: shell.clone(),
                })
            }
            ClientMessage::StopServer { server_id } => Message::StopServer(ServerRequest {
                server_id: server_id.0,
            }),
            ClientMessage::RevokeDevice { key } => {
                Message::RevokeDevice(super::RevokeDevice { key: key.clone() })
            }
            ClientMessage::ConnectedDevices => Message::ConnectedDevices(Empty {}),
            ClientMessage::ServerAdmin { server_id, command } => {
                Message::ServerAdmin(ServerAdminRequest {
                    server_id: server_id.0,
                    command: Some(command.into()),
                })
            }
            ClientMessage::Detach => Message::Detach(Empty {}),
            ClientMessage::Unknown => return Err("an unknown message cannot be sent".into()),
        };
        Ok(Self {
            message: Some(message),
        })
    }
}

impl TryFrom<super::ClientMessage> for ClientMessage {
    type Error = WireError;

    /// An unknown value anywhere inside becomes [`ClientMessage::Unknown`], which the
    /// Server answers without closing the connection; only malformed input is an error.
    fn try_from(message: super::ClientMessage) -> WireResult<Self> {
        match decode_client_message(message) {
            Err(WireError::Unknown) => Ok(Self::Unknown),
            other => other,
        }
    }
}

fn decode_client_message(message: super::ClientMessage) -> WireResult<ClientMessage> {
    {
        use client_message::Message;
        let session = |request: SessionRequest| SessionId(request.session_id);
        Ok(match member(message.message)? {
            Message::SnapshotRequest(request) => ClientMessage::SnapshotRequest {
                session_id: session(request),
            },
            Message::OverviewRequest(request) => ClientMessage::OverviewRequest {
                session_id: session(request),
            },
            Message::Subscribe(subscribe) => ClientMessage::Subscribe {
                session_id: SessionId(subscribe.session_id),
                after_sequence: subscribe.after_sequence,
            },
            Message::Ping(ping) => ClientMessage::Ping {
                server_id: ServerId(ping.server_id),
                nonce: ping.nonce,
            },
            Message::AcquireControl(request) => ClientMessage::AcquireControl {
                session_id: session(request),
            },
            Message::ReleaseControl(request) => ClientMessage::ReleaseControl {
                session_id: session(request),
            },
            Message::Layout(layout) => ClientMessage::Layout {
                server_id: ServerId(layout.server_id),
                session_id: SessionId(layout.session_id),
                request_id: layout.request_id,
                command: required(layout.command, "layout command")?.try_into()?,
            },
            Message::Terminal(terminal) => ClientMessage::Terminal {
                server_id: ServerId(terminal.server_id),
                session_id: SessionId(terminal.session_id),
                pane_id: pane(terminal.pane_id),
                command: TerminalCommand::try_from(required(
                    terminal.command,
                    "terminal command",
                )?)?,
            },
            Message::PasteImage(image) => ClientMessage::PasteImage {
                server_id: ServerId(image.server_id),
                session_id: SessionId(image.session_id),
                pane_id: pane(image.pane_id),
                format: match enum_value(image.format, "image format")? {
                    super::ClipboardImageFormat::Unspecified => {
                        unreachable!("enum_value rejects 0")
                    }
                    super::ClipboardImageFormat::Png => ClipboardImageFormat::Png,
                    super::ClipboardImageFormat::Jpeg => ClipboardImageFormat::Jpeg,
                    super::ClipboardImageFormat::Gif => ClipboardImageFormat::Gif,
                    super::ClipboardImageFormat::Webp => ClipboardImageFormat::Webp,
                    super::ClipboardImageFormat::Bmp => ClipboardImageFormat::Bmp,
                },
                bytes: image.bytes,
            },
            Message::ReadPane(read) => ClientMessage::ReadPane {
                server_id: ServerId(read.server_id),
                session_id: SessionId(read.session_id),
                pane_id: pane(read.pane_id),
                lines: read.lines,
            },
            Message::GitDiff(diff) => ClientMessage::GitDiff {
                server_id: ServerId(diff.server_id),
                session_id: SessionId(diff.session_id),
                request_id: diff.request_id,
                workspace_id: workspace(diff.workspace_id),
                path: relative_path(diff.path)?,
                against: match enum_value(diff.against, "diff base")? {
                    super::DiffBase::Unspecified => unreachable!("enum_value rejects 0"),
                    super::DiffBase::Head => DiffBase::Head,
                },
            },
            Message::ListDirectory(request) => ClientMessage::ListDirectory {
                server_id: ServerId(request.server_id),
                session_id: SessionId(request.session_id),
                request_id: request.request_id,
                workspace_id: workspace(request.workspace_id),
                path: relative_path(request.path)?,
            },
            Message::ReadFile(request) => ClientMessage::ReadFile {
                server_id: ServerId(request.server_id),
                session_id: SessionId(request.session_id),
                request_id: request.request_id,
                workspace_id: workspace(request.workspace_id),
                path: relative_path(request.path)?,
            },
            Message::Agent(agent) => ClientMessage::Agent {
                server_id: ServerId(agent.server_id),
                session_id: SessionId(agent.session_id),
                command: required(agent.command, "agent command")?.try_into()?,
            },
            Message::SetServerSettings(settings) => ClientMessage::SetServerSettings {
                server_id: ServerId(settings.server_id),
                shell: settings.shell,
            },
            Message::StopServer(request) => ClientMessage::StopServer {
                server_id: ServerId(request.server_id),
            },
            Message::RevokeDevice(revoke) => ClientMessage::RevokeDevice { key: revoke.key },
            Message::ConnectedDevices(_) => ClientMessage::ConnectedDevices,
            Message::ServerAdmin(admin) => ClientMessage::ServerAdmin {
                server_id: ServerId(admin.server_id),
                command: required(admin.command, "admin command")?.try_into()?,
            },
            Message::Detach(_) => ClientMessage::Detach,
        })
    }
}

impl From<&ServerAdminCommand> for super::ServerAdminCommand {
    fn from(command: &ServerAdminCommand) -> Self {
        use server_admin_command::Command;
        Self {
            command: Some(match command {
                ServerAdminCommand::Status => Command::Status(Empty {}),
                ServerAdminCommand::SaveListen { address } => Command::SaveListen(SaveListen {
                    address: address.clone(),
                }),
                ServerAdminCommand::SaveP2p { enabled } => Command::SaveP2p(*enabled),
                ServerAdminCommand::Restart => Command::Restart(Empty {}),
                ServerAdminCommand::Clients => Command::Clients(Empty {}),
                ServerAdminCommand::Invite => Command::Invite(Empty {}),
                ServerAdminCommand::Revoke { key } => Command::Revoke(key.clone()),
            }),
        }
    }
}

impl TryFrom<super::ServerAdminCommand> for ServerAdminCommand {
    type Error = WireError;

    fn try_from(command: super::ServerAdminCommand) -> WireResult<Self> {
        use server_admin_command::Command;
        Ok(match member(command.command)? {
            Command::Status(_) => Self::Status,
            Command::SaveListen(save) => Self::SaveListen {
                address: save.address,
            },
            Command::SaveP2p(enabled) => Self::SaveP2p { enabled },
            Command::Restart(_) => Self::Restart,
            Command::Clients(_) => Self::Clients,
            Command::Invite(_) => Self::Invite,
            Command::Revoke(key) => Self::Revoke { key },
        })
    }
}

impl From<&AgentCommand> for super::AgentCommand {
    fn from(command: &AgentCommand) -> Self {
        use agent_command::Command;
        let states = |states: &[crate::AgentState]| {
            states
                .iter()
                .map(|state| super::AgentState::from(*state) as i32)
                .collect()
        };
        Self {
            command: Some(match command {
                AgentCommand::Available => Command::Available(Empty {}),
                AgentCommand::List => Command::List(Empty {}),
                AgentCommand::Start {
                    name,
                    kind,
                    pane_id,
                    args,
                    timeout_ms,
                } => Command::Start(AgentStart {
                    name: name.clone(),
                    kind: encode_kind(*kind),
                    pane_id: pane_id.as_u64(),
                    args: args.clone(),
                    timeout_ms: *timeout_ms,
                }),
                AgentCommand::Prompt {
                    target,
                    text,
                    until,
                    timeout_ms,
                } => Command::Prompt(AgentPrompt {
                    target: target.clone(),
                    text: text.clone(),
                    until: until.as_ref().map(|until| AgentStates {
                        states: states(until),
                    }),
                    timeout_ms: *timeout_ms,
                }),
                AgentCommand::Wait {
                    target,
                    until,
                    timeout_ms,
                } => Command::Wait(AgentWait {
                    target: target.clone(),
                    until: states(until),
                    timeout_ms: *timeout_ms,
                }),
                AgentCommand::Hooks { agent, action } => Command::Hooks(AgentHooks {
                    agent: encode_kind(*agent),
                    action: super::HooksAction::from(*action) as i32,
                }),
            }),
        }
    }
}

impl TryFrom<super::AgentCommand> for AgentCommand {
    type Error = WireError;

    fn try_from(command: super::AgentCommand) -> WireResult<Self> {
        use agent_command::Command;
        let states = |states: Vec<i32>| {
            states
                .into_iter()
                .map(agent_state)
                .collect::<WireResult<Vec<_>>>()
        };
        Ok(match member(command.command)? {
            Command::Available(_) => Self::Available,
            Command::List(_) => Self::List,
            Command::Start(start) => Self::Start {
                name: start.name,
                kind: command_agent_kind(start.kind)?,
                pane_id: pane(start.pane_id),
                args: start.args,
                timeout_ms: start.timeout_ms,
            },
            Command::Prompt(prompt) => Self::Prompt {
                target: prompt.target,
                text: prompt.text,
                until: prompt.until.map(|until| states(until.states)).transpose()?,
                timeout_ms: prompt.timeout_ms,
            },
            Command::Wait(wait) => Self::Wait {
                target: wait.target,
                until: states(wait.until)?,
                timeout_ms: wait.timeout_ms,
            },
            Command::Hooks(hooks) => Self::Hooks {
                agent: command_agent_kind(hooks.agent)?,
                action: hooks_action(hooks.action)?,
            },
        })
    }
}

impl TryFrom<&LayoutCommand> for super::LayoutCommand {
    type Error = String;

    fn try_from(command: &LayoutCommand) -> Result<Self, String> {
        use layout_command::Command;
        let workspace_target = |workspace_id: &WorkspaceId| WorkspaceTarget {
            workspace_id: workspace_id.as_u64(),
        };
        let tab_target = |tab_id: &TabId| TabTarget {
            tab_id: tab_id.as_u64(),
        };
        let pane_target = |pane_id: &PaneId| PaneTarget {
            pane_id: pane_id.as_u64(),
        };
        let direction =
            |direction: &crate::PaneDirection| super::PaneDirection::from(*direction) as i32;
        let workspace_path = |workspace_id: &WorkspaceId, path: &RelativePathBuf| WorkspacePath {
            workspace_id: workspace_id.as_u64(),
            path: path.to_string(),
        };
        let command = match command {
            LayoutCommand::CreateWorkspace {
                root_directory,
                name,
            } => Command::CreateWorkspace(super::CreateWorkspace {
                root_directory: path_text(root_directory)?,
                name: name.clone(),
            }),
            LayoutCommand::CreateWorktree {
                parent_workspace_id,
                branch,
            } => Command::CreateWorktree(super::CreateWorktree {
                parent_workspace_id: parent_workspace_id.as_u64(),
                branch: branch.clone(),
            }),
            LayoutCommand::OpenWorktree {
                parent_workspace_id,
                root_directory,
            } => Command::OpenWorktree(super::OpenWorktree {
                parent_workspace_id: parent_workspace_id.as_u64(),
                root_directory: path_text(root_directory)?,
            }),
            LayoutCommand::RemoveWorktree { workspace_id } => {
                Command::RemoveWorktree(workspace_target(workspace_id))
            }
            LayoutCommand::CreateTab {
                workspace_id,
                name,
                cwd_from,
            } => Command::CreateTab(super::CreateTab {
                workspace_id: workspace_id.as_u64(),
                name: name.clone(),
                cwd_from: cwd_from.map(PaneId::as_u64),
            }),
            LayoutCommand::RenameWorkspace { workspace_id, name } => {
                Command::RenameWorkspace(super::RenameWorkspace {
                    workspace_id: workspace_id.as_u64(),
                    name: name.clone(),
                })
            }
            LayoutCommand::RenameTab { tab_id, name } => Command::RenameTab(super::RenameTab {
                tab_id: tab_id.as_u64(),
                name: name.clone(),
            }),
            LayoutCommand::ActivateWorkspace { workspace_id } => {
                Command::ActivateWorkspace(workspace_target(workspace_id))
            }
            LayoutCommand::ActivateTab { tab_id } => Command::ActivateTab(tab_target(tab_id)),
            LayoutCommand::MoveWorkspace {
                workspace_id,
                target_index,
            } => Command::MoveWorkspace(super::MoveWorkspace {
                workspace_id: workspace_id.as_u64(),
                target_index: *target_index,
            }),
            LayoutCommand::MoveTab {
                tab_id,
                target_index,
            } => Command::MoveTab(super::MoveTab {
                tab_id: tab_id.as_u64(),
                target_index: *target_index,
            }),
            LayoutCommand::SplitPane {
                pane_id,
                direction,
                focus,
            } => Command::SplitPane(super::SplitPane {
                pane_id: pane_id.as_u64(),
                direction: super::SplitDirection::from(*direction) as i32,
                focus: *focus,
            }),
            LayoutCommand::FocusPane { pane_id } => Command::FocusPane(pane_target(pane_id)),
            LayoutCommand::FocusPaneDirection {
                pane_id,
                direction: target,
            } => Command::FocusPaneDirection(PaneInDirection {
                pane_id: pane_id.as_u64(),
                direction: direction(target),
            }),
            LayoutCommand::ResizePane {
                pane_id,
                direction: target,
                amount,
            } => Command::ResizePane(super::ResizePane {
                pane_id: pane_id.as_u64(),
                direction: direction(target),
                amount: *amount,
            }),
            LayoutCommand::SetSplitRatios { tab_id, ratios } => {
                Command::SetSplitRatios(super::SetSplitRatios {
                    tab_id: tab_id.as_u64(),
                    ratios: ratios.clone(),
                })
            }
            LayoutCommand::SwapPane {
                pane_id,
                direction: target,
            } => Command::SwapPane(PaneInDirection {
                pane_id: pane_id.as_u64(),
                direction: direction(target),
            }),
            LayoutCommand::SwapPanes { pane_id, other } => Command::SwapPanes(super::SwapPanes {
                pane_id: pane_id.as_u64(),
                other: other.as_u64(),
            }),
            LayoutCommand::MovePane {
                pane_id,
                target_pane_id,
                side,
            } => Command::MovePane(super::MovePane {
                pane_id: pane_id.as_u64(),
                target_pane_id: target_pane_id.as_u64(),
                side: direction(side),
            }),
            LayoutCommand::TogglePaneZoom { pane_id } => {
                Command::TogglePaneZoom(pane_target(pane_id))
            }
            LayoutCommand::ClosePane { pane_id } => Command::ClosePane(pane_target(pane_id)),
            LayoutCommand::CloseTab { tab_id } => Command::CloseTab(tab_target(tab_id)),
            LayoutCommand::CloseWorkspace { workspace_id } => {
                Command::CloseWorkspace(workspace_target(workspace_id))
            }
            LayoutCommand::ShowDiff { workspace_id, path } => {
                Command::ShowDiff(workspace_path(workspace_id, path))
            }
            LayoutCommand::ShowFile { workspace_id, path } => {
                Command::ShowFile(workspace_path(workspace_id, path))
            }
        };
        Ok(Self {
            command: Some(command),
        })
    }
}

impl TryFrom<super::LayoutCommand> for LayoutCommand {
    type Error = WireError;

    fn try_from(command: super::LayoutCommand) -> WireResult<Self> {
        use layout_command::Command;
        Ok(match member(command.command)? {
            Command::CreateWorkspace(create) => Self::CreateWorkspace {
                root_directory: PathBuf::from(create.root_directory),
                name: create.name,
            },
            Command::CreateWorktree(create) => Self::CreateWorktree {
                parent_workspace_id: workspace(create.parent_workspace_id),
                branch: create.branch,
            },
            Command::OpenWorktree(open) => Self::OpenWorktree {
                parent_workspace_id: workspace(open.parent_workspace_id),
                root_directory: PathBuf::from(open.root_directory),
            },
            Command::RemoveWorktree(target) => Self::RemoveWorktree {
                workspace_id: workspace(target.workspace_id),
            },
            Command::CreateTab(create) => Self::CreateTab {
                workspace_id: workspace(create.workspace_id),
                name: create.name,
                cwd_from: create.cwd_from.map(pane),
            },
            Command::RenameWorkspace(rename) => Self::RenameWorkspace {
                workspace_id: workspace(rename.workspace_id),
                name: rename.name,
            },
            Command::RenameTab(rename) => Self::RenameTab {
                tab_id: tab(rename.tab_id),
                name: rename.name,
            },
            Command::ActivateWorkspace(target) => Self::ActivateWorkspace {
                workspace_id: workspace(target.workspace_id),
            },
            Command::ActivateTab(target) => Self::ActivateTab {
                tab_id: tab(target.tab_id),
            },
            Command::MoveWorkspace(target) => Self::MoveWorkspace {
                workspace_id: workspace(target.workspace_id),
                target_index: target.target_index,
            },
            Command::MoveTab(target) => Self::MoveTab {
                tab_id: tab(target.tab_id),
                target_index: target.target_index,
            },
            Command::SplitPane(split) => Self::SplitPane {
                pane_id: pane(split.pane_id),
                direction: split_direction(split.direction)?,
                focus: split.focus,
            },
            Command::FocusPane(target) => Self::FocusPane {
                pane_id: pane(target.pane_id),
            },
            Command::FocusPaneDirection(target) => Self::FocusPaneDirection {
                pane_id: pane(target.pane_id),
                direction: pane_direction(target.direction)?,
            },
            Command::ResizePane(resize) => Self::ResizePane {
                pane_id: pane(resize.pane_id),
                direction: pane_direction(resize.direction)?,
                amount: resize.amount,
            },
            Command::SetSplitRatios(ratios) => Self::SetSplitRatios {
                tab_id: tab(ratios.tab_id),
                ratios: ratios.ratios,
            },
            Command::SwapPane(target) => Self::SwapPane {
                pane_id: pane(target.pane_id),
                direction: pane_direction(target.direction)?,
            },
            Command::SwapPanes(swap) => Self::SwapPanes {
                pane_id: pane(swap.pane_id),
                other: pane(swap.other),
            },
            Command::MovePane(r#move) => Self::MovePane {
                pane_id: pane(r#move.pane_id),
                target_pane_id: pane(r#move.target_pane_id),
                side: pane_direction(r#move.side)?,
            },
            Command::TogglePaneZoom(target) => Self::TogglePaneZoom {
                pane_id: pane(target.pane_id),
            },
            Command::ClosePane(target) => Self::ClosePane {
                pane_id: pane(target.pane_id),
            },
            Command::CloseTab(target) => Self::CloseTab {
                tab_id: tab(target.tab_id),
            },
            Command::CloseWorkspace(target) => Self::CloseWorkspace {
                workspace_id: workspace(target.workspace_id),
            },
            Command::ShowDiff(target) => Self::ShowDiff {
                workspace_id: workspace(target.workspace_id),
                path: relative_path(target.path)?,
            },
            Command::ShowFile(target) => Self::ShowFile {
                workspace_id: workspace(target.workspace_id),
                path: relative_path(target.path)?,
            },
        })
    }
}

impl From<&LayoutResult> for super::LayoutResult {
    fn from(result: &LayoutResult) -> Self {
        use layout_result::Result as Wire;
        Self {
            result: Some(match result {
                LayoutResult::Changed => Wire::Changed(Empty {}),
                LayoutResult::WorkspaceCreated {
                    workspace_id,
                    tab_id,
                    pane_id,
                } => Wire::WorkspaceCreated(super::WorkspaceCreated {
                    workspace_id: workspace_id.as_u64(),
                    tab_id: tab_id.as_u64(),
                    pane_id: pane_id.as_u64(),
                }),
                LayoutResult::TabCreated { tab_id, pane_id } => {
                    Wire::TabCreated(super::TabCreated {
                        tab_id: tab_id.as_u64(),
                        pane_id: pane_id.as_u64(),
                    })
                }
                LayoutResult::PaneCreated { pane_id } => Wire::PaneCreated(PaneTarget {
                    pane_id: pane_id.as_u64(),
                }),
                LayoutResult::WorkspaceOpened { workspace_id } => {
                    Wire::WorkspaceOpened(WorkspaceTarget {
                        workspace_id: workspace_id.as_u64(),
                    })
                }
                LayoutResult::DiffShown { tab_id } => Wire::DiffShown(TabTarget {
                    tab_id: tab_id.as_u64(),
                }),
                LayoutResult::FileShown { tab_id } => Wire::FileShown(TabTarget {
                    tab_id: tab_id.as_u64(),
                }),
            }),
        }
    }
}

impl TryFrom<super::LayoutResult> for LayoutResult {
    type Error = WireError;

    fn try_from(result: super::LayoutResult) -> WireResult<Self> {
        use layout_result::Result as Wire;
        Ok(match member(result.result)? {
            Wire::Changed(_) => Self::Changed,
            Wire::WorkspaceCreated(created) => Self::WorkspaceCreated {
                workspace_id: workspace(created.workspace_id),
                tab_id: tab(created.tab_id),
                pane_id: pane(created.pane_id),
            },
            Wire::TabCreated(created) => Self::TabCreated {
                tab_id: tab(created.tab_id),
                pane_id: pane(created.pane_id),
            },
            Wire::PaneCreated(target) => Self::PaneCreated {
                pane_id: pane(target.pane_id),
            },
            Wire::WorkspaceOpened(target) => Self::WorkspaceOpened {
                workspace_id: workspace(target.workspace_id),
            },
            Wire::DiffShown(target) => Self::DiffShown {
                tab_id: tab(target.tab_id),
            },
            Wire::FileShown(target) => Self::FileShown {
                tab_id: tab(target.tab_id),
            },
        })
    }
}

impl TryFrom<&BootstrapHeader> for super::BootstrapHeader {
    type Error = String;

    fn try_from(header: &BootstrapHeader) -> Result<Self, String> {
        Ok(Self {
            server_id: header.server_id.0,
            runtime_epoch: Some(encode_epoch(header.runtime_epoch)),
            session_id: header.session_id.0,
            sequence: header.sequence,
            snapshot: encode_snapshot(&header.snapshot)?,
            settings: Some(encode_settings(&header.settings)),
            batch_count: header.batch_count,
        })
    }
}

impl TryFrom<super::BootstrapHeader> for BootstrapHeader {
    type Error = WireError;

    fn try_from(header: super::BootstrapHeader) -> WireResult<Self> {
        Ok(Self {
            server_id: ServerId(header.server_id),
            runtime_epoch: decode_epoch(header.runtime_epoch)?,
            session_id: SessionId(header.session_id),
            sequence: header.sequence,
            snapshot: decode_snapshot(header.snapshot)?,
            settings: decode_settings(required(header.settings, "Server settings")?),
            batch_count: header.batch_count,
        })
    }
}

impl TryFrom<&SessionOverview> for super::SessionOverview {
    type Error = String;

    fn try_from(overview: &SessionOverview) -> Result<Self, String> {
        Ok(Self {
            server_id: overview.server_id.0,
            runtime_epoch: Some(encode_epoch(overview.runtime_epoch)),
            session_id: overview.session_id.0,
            sequence: overview.sequence,
            snapshot: encode_snapshot(&overview.snapshot)?,
            terminals: encode_metadata(&overview.terminals),
            agents: overview.agents.iter().map(encode_pane_agent).collect(),
            zoomed_panes: pane_ids(&overview.zoomed_panes),
        })
    }
}

impl TryFrom<super::SessionOverview> for SessionOverview {
    type Error = WireError;

    fn try_from(overview: super::SessionOverview) -> WireResult<Self> {
        Ok(Self {
            server_id: ServerId(overview.server_id),
            runtime_epoch: decode_epoch(overview.runtime_epoch)?,
            session_id: SessionId(overview.session_id),
            sequence: overview.sequence,
            snapshot: decode_snapshot(overview.snapshot)?,
            terminals: decode_metadata(overview.terminals),
            agents: overview
                .agents
                .into_iter()
                .map(decode_pane_agent)
                .collect::<WireResult<_>>()?,
            zoomed_panes: overview.zoomed_panes.into_iter().map(pane).collect(),
        })
    }
}

impl From<&BootstrapRecord> for super::BootstrapRecord {
    fn from(record: &BootstrapRecord) -> Self {
        use bootstrap_record::Record;
        Self {
            record: Some(match record {
                BootstrapRecord::Terminal(terminal) => {
                    Record::Terminal(super::PaneTerminalSnapshot {
                        pane_id: terminal.pane_id.as_u64(),
                        view: Some((&terminal.view).into()),
                        exited: terminal.exited,
                        title: terminal.title.clone(),
                        attention: terminal.attention,
                    })
                }
                BootstrapRecord::Agent(agent) => Record::Agent(encode_pane_agent(agent)),
                BootstrapRecord::WorkspaceGit(git) => Record::WorkspaceGit(encode_git(git)),
                BootstrapRecord::ZoomedPane(pane) => Record::ZoomedPane(pane.as_u64()),
            }),
        }
    }
}

/// `None` is a record kind this build does not know, which the Bootstrap skips.
pub(crate) fn decode_bootstrap_record(
    record: super::BootstrapRecord,
) -> WireResult<Option<BootstrapRecord>> {
    use bootstrap_record::Record;
    let Some(record) = record.record else {
        return Ok(None);
    };
    Ok(Some(match record {
        Record::Terminal(terminal) => BootstrapRecord::Terminal(PaneTerminalSnapshot {
            pane_id: pane(terminal.pane_id),
            view: TerminalView::try_from(required(terminal.view, "terminal view")?)?,
            exited: terminal.exited,
            title: terminal.title,
            attention: terminal.attention,
        }),
        Record::Agent(agent) => BootstrapRecord::Agent(decode_pane_agent(agent)?),
        Record::WorkspaceGit(git) => BootstrapRecord::WorkspaceGit(decode_git(git)?),
        Record::ZoomedPane(id) => BootstrapRecord::ZoomedPane(pane(id)),
    }))
}

impl From<&PaneTerminalFrame> for super::PaneTerminalFrame {
    fn from(frame: &PaneTerminalFrame) -> Self {
        Self {
            pane_id: frame.pane_id.as_u64(),
            frame: Some((&frame.frame).into()),
        }
    }
}

impl TryFrom<super::PaneTerminalFrame> for PaneTerminalFrame {
    type Error = WireError;

    fn try_from(frame: super::PaneTerminalFrame) -> WireResult<Self> {
        Ok(Self {
            pane_id: pane(frame.pane_id),
            frame: TerminalViewFrame::try_from(required(frame.frame, "terminal frame")?)?,
        })
    }
}

impl TryFrom<&SessionEvent> for super::SessionEvent {
    type Error = String;

    fn try_from(event: &SessionEvent) -> Result<Self, String> {
        use session_event::Event;
        let event = match event {
            SessionEvent::LayoutChanged {
                snapshot,
                zoomed_panes,
            } => Event::LayoutChanged(super::LayoutChanged {
                snapshot: encode_snapshot(snapshot)?,
                zoomed_panes: pane_ids(zoomed_panes),
            }),
            SessionEvent::TerminalExited { pane_id } => Event::TerminalExited(PaneTarget {
                pane_id: pane_id.as_u64(),
            }),
            SessionEvent::AgentChanged { pane_id, agent } => {
                Event::AgentChanged(super::AgentChanged {
                    pane_id: pane_id.as_u64(),
                    agent: agent.as_ref().map(Into::into),
                })
            }
            SessionEvent::WorkspaceGitChanged { workspace_id, git } => {
                Event::WorkspaceGitChanged(super::WorkspaceGitChanged {
                    workspace_id: workspace_id.as_u64(),
                    git: git.as_ref().map(encode_git),
                })
            }
            SessionEvent::WorkspaceFilesChanged { workspace_id } => {
                Event::WorkspaceFilesChanged(WorkspaceTarget {
                    workspace_id: workspace_id.as_u64(),
                })
            }
            SessionEvent::TerminalTitleChanged { pane_id, title } => {
                Event::TerminalTitleChanged(super::TerminalTitleChanged {
                    pane_id: pane_id.as_u64(),
                    title: title.clone(),
                })
            }
            SessionEvent::TerminalAttentionChanged { pane_id, attention } => {
                Event::TerminalAttentionChanged(super::TerminalAttentionChanged {
                    pane_id: pane_id.as_u64(),
                    attention: *attention,
                })
            }
            SessionEvent::ServerSettingsChanged { settings } => {
                Event::ServerSettingsChanged(encode_settings(settings))
            }
            SessionEvent::Activated {
                workspace_id,
                tab_id,
            } => Event::Activated(super::Activated {
                workspace_id: workspace_id.as_u64(),
                tab_id: tab_id.map(TabId::as_u64),
            }),
            SessionEvent::Omitted => Event::Omitted(Empty {}),
        };
        Ok(Self { event: Some(event) })
    }
}

impl TryFrom<super::SessionEvent> for SessionEvent {
    type Error = WireError;

    fn try_from(event: super::SessionEvent) -> WireResult<Self> {
        use session_event::Event;
        Ok(match member(event.event)? {
            Event::LayoutChanged(changed) => Self::LayoutChanged {
                snapshot: decode_snapshot(changed.snapshot)?,
                zoomed_panes: changed.zoomed_panes.into_iter().map(pane).collect(),
            },
            Event::TerminalExited(target) => Self::TerminalExited {
                pane_id: pane(target.pane_id),
            },
            Event::AgentChanged(changed) => Self::AgentChanged {
                pane_id: pane(changed.pane_id),
                agent: changed.agent.map(TryInto::try_into).transpose()?,
            },
            Event::WorkspaceGitChanged(changed) => Self::WorkspaceGitChanged {
                workspace_id: workspace(changed.workspace_id),
                git: changed.git.map(decode_git).transpose()?,
            },
            Event::WorkspaceFilesChanged(target) => Self::WorkspaceFilesChanged {
                workspace_id: workspace(target.workspace_id),
            },
            Event::TerminalTitleChanged(changed) => Self::TerminalTitleChanged {
                pane_id: pane(changed.pane_id),
                title: changed.title,
            },
            Event::TerminalAttentionChanged(changed) => Self::TerminalAttentionChanged {
                pane_id: pane(changed.pane_id),
                attention: changed.attention,
            },
            Event::ServerSettingsChanged(settings) => Self::ServerSettingsChanged {
                settings: decode_settings(settings),
            },
            Event::Activated(activated) => Self::Activated {
                workspace_id: workspace(activated.workspace_id),
                tab_id: activated.tab_id.map(tab),
            },
            Event::Omitted(_) => Self::Omitted,
        })
    }
}

fn encode_agent_info(info: &AgentInfo) -> super::AgentInfo {
    super::AgentInfo {
        pane_id: info.pane_id.as_u64(),
        name: info.name.clone(),
        agent: encode_agent(&info.agent),
        launch_pending: info.launch_pending,
    }
}

fn decode_agent_info(info: super::AgentInfo) -> WireResult<AgentInfo> {
    Ok(AgentInfo {
        pane_id: pane(info.pane_id),
        name: info.name,
        agent: decode_agent(info.agent)?,
        launch_pending: info.launch_pending,
    })
}

fn encode_agent_result(
    result: &Result<AgentResponse, AgentError>,
) -> Result<super::AgentResult, String> {
    use agent_response::Response;
    use agent_result::Result as Wire;
    let result = match result {
        Ok(response) => Wire::Ok(super::AgentResponse {
            response: Some(match response {
                AgentResponse::Available(installations) => {
                    Response::Available(AgentInstallations {
                        installations: installations
                            .iter()
                            .map(super::AgentInstallation::try_from)
                            .collect::<Result<_, _>>()?,
                    })
                }
                AgentResponse::List(agents) => Response::List(AgentInfos {
                    agents: agents.iter().map(encode_agent_info).collect(),
                }),
                AgentResponse::Ready(info) => Response::Ready(encode_agent_info(info)),
                AgentResponse::Hooks(report) => Response::Hooks(report.try_into()?),
            }),
        }),
        Err(error) => Wire::Err(super::AgentError {
            code: error.code.clone(),
            message: error.message.clone(),
        }),
    };
    Ok(super::AgentResult {
        result: Some(result),
    })
}

fn decode_agent_result(
    result: super::AgentResult,
) -> WireResult<Result<AgentResponse, AgentError>> {
    use agent_response::Response;
    use agent_result::Result as Wire;
    Ok(match member(result.result)? {
        Wire::Ok(response) => Ok(match member(response.response)? {
            Response::Available(available) => AgentResponse::Available(
                available
                    .installations
                    .into_iter()
                    .map(AgentInstallation::try_from)
                    .collect::<WireResult<_>>()?,
            ),
            Response::List(list) => AgentResponse::List(
                list.agents
                    .into_iter()
                    .map(decode_agent_info)
                    .collect::<WireResult<_>>()?,
            ),
            Response::Ready(info) => AgentResponse::Ready(decode_agent_info(info)?),
            Response::Hooks(report) => AgentResponse::Hooks(HooksReport::try_from(report)?),
        }),
        Wire::Err(error) => Err(AgentError {
            code: error.code,
            message: error.message,
        }),
    })
}

impl From<&ServerAdminResponse> for super::ServerAdminResponse {
    fn from(response: &ServerAdminResponse) -> Self {
        use server_admin_response::Response;
        Self {
            response: Some(match response {
                ServerAdminResponse::Status {
                    listen,
                    p2p,
                    connected,
                    version,
                    uptime_secs,
                    workspaces,
                    tabs,
                    panes,
                    agents,
                    clients,
                    recent_errors,
                } => Response::Status(ServerStatus {
                    listen: listen.clone(),
                    p2p: *p2p,
                    connected: connected.clone(),
                    version: version.clone(),
                    uptime_secs: *uptime_secs,
                    workspaces: *workspaces,
                    tabs: *tabs,
                    panes: *panes,
                    agents: *agents,
                    clients: *clients,
                    recent_errors: recent_errors
                        .iter()
                        .map(|record| super::ServerLogRecord {
                            at: record.at,
                            level: record.level.clone(),
                            message: record.message.clone(),
                        })
                        .collect(),
                }),
                ServerAdminResponse::ListenSaved { listen } => Response::ListenSaved(SaveListen {
                    address: listen.clone(),
                }),
                ServerAdminResponse::P2pSaved { enabled } => Response::P2pSaved(*enabled),
                ServerAdminResponse::Clients { clients, connected } => {
                    Response::Clients(ServerClients {
                        clients: clients
                            .iter()
                            .map(|client| super::ServerClientInfo {
                                name: client.name.clone(),
                                fingerprint: client.fingerprint.clone(),
                                last_seen: client.last_seen,
                            })
                            .collect(),
                        connected: connected.clone(),
                    })
                }
                ServerAdminResponse::Invite {
                    tcp,
                    p2p,
                    expires_in_secs,
                } => Response::Invite(ServerInvite {
                    tcp: tcp.clone(),
                    p2p: p2p.clone(),
                    expires_in_secs: *expires_in_secs,
                }),
                ServerAdminResponse::Revoked { disconnected } => Response::Revoked(*disconnected),
            }),
        }
    }
}

impl TryFrom<super::ServerAdminResponse> for ServerAdminResponse {
    type Error = WireError;

    fn try_from(response: super::ServerAdminResponse) -> WireResult<Self> {
        use server_admin_response::Response;
        Ok(match member(response.response)? {
            Response::Status(status) => Self::Status {
                listen: status.listen,
                p2p: status.p2p,
                connected: status.connected,
                version: status.version,
                uptime_secs: status.uptime_secs,
                workspaces: status.workspaces,
                tabs: status.tabs,
                panes: status.panes,
                agents: status.agents,
                clients: status.clients,
                recent_errors: status
                    .recent_errors
                    .into_iter()
                    .map(|record| ServerLogRecord {
                        at: record.at,
                        level: record.level,
                        message: record.message,
                    })
                    .collect(),
            },
            Response::ListenSaved(saved) => Self::ListenSaved {
                listen: saved.address,
            },
            Response::P2pSaved(enabled) => Self::P2pSaved { enabled },
            Response::Clients(clients) => Self::Clients {
                clients: clients
                    .clients
                    .into_iter()
                    .map(|client| ServerClientInfo {
                        name: client.name,
                        fingerprint: client.fingerprint,
                        last_seen: client.last_seen,
                    })
                    .collect(),
                connected: clients.connected,
            },
            Response::Invite(invite) => Self::Invite {
                tcp: invite.tcp,
                p2p: invite.p2p,
                expires_in_secs: invite.expires_in_secs,
            },
            Response::Revoked(disconnected) => Self::Revoked { disconnected },
        })
    }
}

impl TryFrom<&ServerMessage> for super::ServerMessage {
    type Error = String;

    fn try_from(message: &ServerMessage) -> Result<Self, String> {
        use server_message::Message;
        let address = |server_id: &ServerId, session_id: &SessionId| SessionAddress {
            server_id: server_id.0,
            session_id: session_id.0,
        };
        let reason =
            |server_id: &ServerId, session_id: &SessionId, reason: &String| SessionReason {
                server_id: server_id.0,
                session_id: session_id.0,
                reason: reason.clone(),
            };
        let message = match message {
            ServerMessage::Bootstrap(header) => Message::Bootstrap(header.try_into()?),
            ServerMessage::Overview(overview) => Message::Overview(overview.try_into()?),
            ServerMessage::OverviewTerminals {
                server_id,
                session_id,
                terminals,
            } => Message::OverviewTerminals(super::OverviewTerminals {
                server_id: server_id.0,
                session_id: session_id.0,
                terminals: encode_metadata(terminals),
            }),
            ServerMessage::BootstrapBatch(batch) => {
                Message::BootstrapBatch(super::BootstrapBatch {
                    server_id: batch.server_id.0,
                    session_id: batch.session_id.0,
                    batch_index: batch.batch_index,
                    record_index: batch.record_index,
                    chunk_index: batch.chunk_index,
                    chunk_count: batch.chunk_count,
                    payload: batch.payload.clone(),
                })
            }
            ServerMessage::Subscribed {
                server_id,
                session_id,
                sequence,
            } => Message::Subscribed(SessionSequence {
                server_id: server_id.0,
                session_id: session_id.0,
                sequence: *sequence,
            }),
            ServerMessage::SnapshotRejected {
                server_id,
                session_id,
                reason: text,
            } => Message::SnapshotRejected(reason(server_id, session_id, text)),
            ServerMessage::SubscriptionRejected {
                server_id,
                session_id,
                reason: text,
            } => Message::SubscriptionRejected(reason(server_id, session_id, text)),
            ServerMessage::Event {
                server_id,
                session_id,
                sequence,
                event,
            } => Message::Event(SessionEventMessage {
                server_id: server_id.0,
                session_id: session_id.0,
                sequence: *sequence,
                event: Some(event.try_into()?),
            }),
            ServerMessage::TerminalFrame(batch) => {
                Message::TerminalFrame(super::TerminalFrameBatch {
                    server_id: batch.server_id.0,
                    session_id: batch.session_id.0,
                    panes: batch.panes.iter().map(Into::into).collect(),
                })
            }
            ServerMessage::TerminalFrameChunk(chunk) => {
                Message::TerminalFrameChunk(super::TerminalFrameChunk {
                    server_id: chunk.server_id.0,
                    session_id: chunk.session_id.0,
                    pane_id: chunk.pane_id.as_u64(),
                    revision: chunk.revision,
                    chunk_index: chunk.chunk_index,
                    chunk_count: chunk.chunk_count,
                    payload: chunk.payload.clone(),
                })
            }
            ServerMessage::Pong {
                server_id,
                nonce,
                sequence,
            } => Message::Pong(super::Pong {
                server_id: server_id.0,
                nonce: *nonce,
                sequence: *sequence,
            }),
            ServerMessage::ControlGranted {
                server_id,
                session_id,
            } => Message::ControlGranted(address(server_id, session_id)),
            ServerMessage::ControlReleased {
                server_id,
                session_id,
            } => Message::ControlReleased(address(server_id, session_id)),
            ServerMessage::ControlDenied {
                server_id,
                session_id,
                reason: text,
            } => Message::ControlDenied(reason(server_id, session_id, text)),
            ServerMessage::LayoutRejected {
                server_id,
                session_id,
                request_id,
                reason,
            } => Message::LayoutRejected(super::LayoutRejected {
                server_id: server_id.0,
                session_id: session_id.0,
                request_id: *request_id,
                reason: reason.clone(),
            }),
            ServerMessage::LayoutApplied {
                server_id,
                session_id,
                request_id,
                sequence,
                result,
            } => Message::LayoutApplied(super::LayoutApplied {
                server_id: server_id.0,
                session_id: session_id.0,
                request_id: *request_id,
                sequence: *sequence,
                result: Some(result.into()),
            }),
            ServerMessage::TerminalCopied { pane_id, text } => {
                Message::TerminalCopied(super::TerminalCopied {
                    pane_id: pane_id.as_u64(),
                    text: text.clone(),
                })
            }
            ServerMessage::PaneText { pane_id, text } => Message::PaneText(super::PaneText {
                pane_id: pane_id.as_u64(),
                text: text.clone(),
            }),
            ServerMessage::GitDiff {
                request_id,
                workspace_id,
                path,
                result,
            } => Message::GitDiff(GitDiffReply {
                request_id: *request_id,
                workspace_id: workspace_id.as_u64(),
                path: path.to_string(),
                result: Some(match result {
                    Ok(diff) => git_diff_reply::Result::Ok(diff.into()),
                    Err(error) => git_diff_reply::Result::Err(error.clone()),
                }),
            }),
            ServerMessage::Directory {
                request_id,
                workspace_id,
                path,
                result,
            } => Message::Directory(DirectoryReply {
                request_id: *request_id,
                workspace_id: workspace_id.as_u64(),
                path: path.to_string(),
                result: Some(match result {
                    Ok(listing) => directory_reply::Result::Ok(listing.into()),
                    Err(error) => directory_reply::Result::Err(error.clone()),
                }),
            }),
            ServerMessage::FileContent {
                request_id,
                workspace_id,
                path,
                result,
            } => Message::FileContent(FileContentReply {
                request_id: *request_id,
                workspace_id: workspace_id.as_u64(),
                path: path.to_string(),
                result: Some(match result {
                    Ok(content) => file_content_reply::Result::Ok(content.into()),
                    Err(error) => file_content_reply::Result::Err(error.clone()),
                }),
            }),
            ServerMessage::AgentResult { result } => {
                Message::AgentResult(encode_agent_result(result)?)
            }
            ServerMessage::TerminalClipboard { pane_id, text } => {
                Message::TerminalClipboard(super::PaneText {
                    pane_id: pane_id.as_u64(),
                    text: text.clone(),
                })
            }
            ServerMessage::ServerStopping => Message::ServerStopping(Empty {}),
            ServerMessage::DevicesRevoked { disconnected } => {
                Message::DevicesRevoked(*disconnected)
            }
            ServerMessage::ConnectedDevices { keys } => {
                Message::ConnectedDevices(super::ConnectedDevices { keys: keys.clone() })
            }
            ServerMessage::ServerAdmin(response) => Message::ServerAdmin(response.into()),
            ServerMessage::Error { message } => Message::Error(message.clone()),
            ServerMessage::Unknown(_) => return Err("an unknown message cannot be sent".into()),
        };
        Ok(Self {
            message: Some(message),
        })
    }
}

impl TryFrom<super::ServerMessage> for ServerMessage {
    type Error = WireError;

    /// An unknown value anywhere inside becomes [`ServerMessage::Unknown`], classified by
    /// the stream it arrived on; only malformed input is an error.
    fn try_from(message: super::ServerMessage) -> WireResult<Self> {
        use server_message::Message;
        let Some(message) = message.message else {
            return Ok(Self::Unknown(UnknownMessage::Reply));
        };
        let unknown = match &message {
            Message::Event(event) => UnknownMessage::Event {
                sequence: event.sequence,
            },
            Message::TerminalFrame(_) | Message::TerminalFrameChunk(_) => {
                UnknownMessage::TerminalFrame
            }
            _ => UnknownMessage::Reply,
        };
        match decode_server_message(message) {
            Err(WireError::Unknown) => Ok(Self::Unknown(unknown)),
            other => other,
        }
    }
}

fn decode_server_message(message: server_message::Message) -> WireResult<ServerMessage> {
    use server_message::Message;
    let address =
        |address: SessionAddress| (ServerId(address.server_id), SessionId(address.session_id));
    Ok(match message {
        Message::Bootstrap(header) => ServerMessage::Bootstrap(header.try_into()?),
        Message::Overview(overview) => ServerMessage::Overview(overview.try_into()?),
        Message::OverviewTerminals(terminals) => ServerMessage::OverviewTerminals {
            server_id: ServerId(terminals.server_id),
            session_id: SessionId(terminals.session_id),
            terminals: decode_metadata(terminals.terminals),
        },
        Message::BootstrapBatch(batch) => ServerMessage::BootstrapBatch(BootstrapBatch {
            server_id: ServerId(batch.server_id),
            session_id: SessionId(batch.session_id),
            batch_index: batch.batch_index,
            record_index: batch.record_index,
            chunk_index: batch.chunk_index,
            chunk_count: batch.chunk_count,
            payload: batch.payload,
        }),
        Message::Subscribed(subscribed) => ServerMessage::Subscribed {
            server_id: ServerId(subscribed.server_id),
            session_id: SessionId(subscribed.session_id),
            sequence: subscribed.sequence,
        },
        Message::SnapshotRejected(rejected) => ServerMessage::SnapshotRejected {
            server_id: ServerId(rejected.server_id),
            session_id: SessionId(rejected.session_id),
            reason: rejected.reason,
        },
        Message::SubscriptionRejected(rejected) => ServerMessage::SubscriptionRejected {
            server_id: ServerId(rejected.server_id),
            session_id: SessionId(rejected.session_id),
            reason: rejected.reason,
        },
        Message::Event(event) => ServerMessage::Event {
            server_id: ServerId(event.server_id),
            session_id: SessionId(event.session_id),
            sequence: event.sequence,
            event: required(event.event, "event")?.try_into()?,
        },
        Message::TerminalFrame(batch) => ServerMessage::TerminalFrame(TerminalFrameBatch {
            server_id: ServerId(batch.server_id),
            session_id: SessionId(batch.session_id),
            panes: batch
                .panes
                .into_iter()
                .map(PaneTerminalFrame::try_from)
                .collect::<WireResult<_>>()?,
        }),
        Message::TerminalFrameChunk(chunk) => {
            ServerMessage::TerminalFrameChunk(TerminalFrameChunk {
                server_id: ServerId(chunk.server_id),
                session_id: SessionId(chunk.session_id),
                pane_id: pane(chunk.pane_id),
                revision: chunk.revision,
                chunk_index: chunk.chunk_index,
                chunk_count: chunk.chunk_count,
                payload: chunk.payload,
            })
        }
        Message::Pong(pong) => ServerMessage::Pong {
            server_id: ServerId(pong.server_id),
            nonce: pong.nonce,
            sequence: pong.sequence,
        },
        Message::ControlGranted(granted) => {
            let (server_id, session_id) = address(granted);
            ServerMessage::ControlGranted {
                server_id,
                session_id,
            }
        }
        Message::ControlReleased(released) => {
            let (server_id, session_id) = address(released);
            ServerMessage::ControlReleased {
                server_id,
                session_id,
            }
        }
        Message::ControlDenied(denied) => ServerMessage::ControlDenied {
            server_id: ServerId(denied.server_id),
            session_id: SessionId(denied.session_id),
            reason: denied.reason,
        },
        Message::LayoutRejected(rejected) => ServerMessage::LayoutRejected {
            server_id: ServerId(rejected.server_id),
            session_id: SessionId(rejected.session_id),
            request_id: rejected.request_id,
            reason: rejected.reason,
        },
        Message::LayoutApplied(applied) => ServerMessage::LayoutApplied {
            server_id: ServerId(applied.server_id),
            session_id: SessionId(applied.session_id),
            request_id: applied.request_id,
            sequence: applied.sequence,
            result: required(applied.result, "layout result")?.try_into()?,
        },
        Message::TerminalCopied(copied) => ServerMessage::TerminalCopied {
            pane_id: pane(copied.pane_id),
            text: copied.text,
        },
        Message::PaneText(text) => ServerMessage::PaneText {
            pane_id: pane(text.pane_id),
            text: text.text,
        },
        Message::GitDiff(reply) => ServerMessage::GitDiff {
            request_id: reply.request_id,
            workspace_id: workspace(reply.workspace_id),
            path: relative_path(reply.path)?,
            result: match member(reply.result)? {
                git_diff_reply::Result::Ok(diff) => Ok(FileDiff::try_from(diff)?),
                git_diff_reply::Result::Err(error) => Err(error),
            },
        },
        Message::Directory(reply) => ServerMessage::Directory {
            request_id: reply.request_id,
            workspace_id: workspace(reply.workspace_id),
            path: relative_path(reply.path)?,
            result: match member(reply.result)? {
                directory_reply::Result::Ok(listing) => Ok(DirectoryListing::try_from(listing)?),
                directory_reply::Result::Err(error) => Err(error),
            },
        },
        Message::FileContent(reply) => ServerMessage::FileContent {
            request_id: reply.request_id,
            workspace_id: workspace(reply.workspace_id),
            path: relative_path(reply.path)?,
            result: match member(reply.result)? {
                file_content_reply::Result::Ok(content) => Ok(FileContent::try_from(content)?),
                file_content_reply::Result::Err(error) => Err(error),
            },
        },
        Message::AgentResult(result) => ServerMessage::AgentResult {
            result: decode_agent_result(result)?,
        },
        Message::TerminalClipboard(text) => ServerMessage::TerminalClipboard {
            pane_id: pane(text.pane_id),
            text: text.text,
        },
        Message::ServerStopping(_) => ServerMessage::ServerStopping,
        Message::DevicesRevoked(disconnected) => ServerMessage::DevicesRevoked { disconnected },
        Message::ConnectedDevices(devices) => {
            ServerMessage::ConnectedDevices { keys: devices.keys }
        }
        Message::ServerAdmin(response) => ServerMessage::ServerAdmin(response.try_into()?),
        Message::Error(message) => ServerMessage::Error { message },
    })
}
