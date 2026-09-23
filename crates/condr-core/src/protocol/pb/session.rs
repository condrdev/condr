//! The Session Snapshot, and the agent, Git and file types Session messages carry.

use super::*;
use crate::snapshot::{
    LayoutNodeSnapshot, LayoutSnapshot, MAX_SNAPSHOT_LAYOUT_NODES, MAX_SNAPSHOT_PANES,
    MAX_SNAPSHOT_TABS, MAX_SNAPSHOT_WORKSPACES, PaneSnapshot, TabContentSnapshot, TabSnapshot,
    WorkspaceSnapshot,
};
use crate::{
    AgentKind, AgentResume, AgentSnapshot, AgentState, DiffHunk, DiffLine, DiffLineKind,
    DirectoryEntry, DirectoryListing, FileContent, FileDiff, FileDiffContent, FileKind,
    GitChangeEntry, GitChangeStatus, GitChanges, GitDiffStat, GitUpstream, PaneDirection, PaneId,
    SessionSnapshot, SplitDirection, TabId, WorkspaceId, WorktreeAssociation,
    agent_discovery::AgentInstallation,
    agent_hooks::{HooksAction, HooksReport, HooksState},
};

impl TryFrom<&SessionSnapshot> for super::SessionSnapshot {
    type Error = String;

    fn try_from(snapshot: &SessionSnapshot) -> Result<Self, String> {
        Ok(Self {
            workspaces: snapshot
                .workspaces
                .iter()
                .map(super::WorkspaceSnapshot::try_from)
                .collect::<Result<_, _>>()?,
        })
    }
}

impl TryFrom<super::SessionSnapshot> for SessionSnapshot {
    type Error = WireError;

    fn try_from(snapshot: super::SessionSnapshot) -> WireResult<Self> {
        bounded(
            snapshot.workspaces.len(),
            MAX_SNAPSHOT_WORKSPACES,
            "Workspaces",
        )?;
        Ok(Self {
            workspaces: snapshot
                .workspaces
                .into_iter()
                .map(WorkspaceSnapshot::try_from)
                .collect::<WireResult<_>>()?,
        })
    }
}

impl TryFrom<&WorkspaceSnapshot> for super::WorkspaceSnapshot {
    type Error = String;

    fn try_from(workspace: &WorkspaceSnapshot) -> Result<Self, String> {
        Ok(Self {
            id: workspace.id.as_u64(),
            name: workspace.name.clone(),
            root_directory: path_text(&workspace.root_directory)?,
            worktree: workspace
                .worktree
                .as_ref()
                .map(|worktree| {
                    Ok::<_, String>(super::WorktreeAssociation {
                        parent_workspace_id: worktree.parent_workspace_id.as_u64(),
                        parent_root_directory: path_text(&worktree.parent_root_directory)?,
                        managed: worktree.managed,
                    })
                })
                .transpose()?,
            tabs: workspace
                .tabs
                .iter()
                .map(super::TabSnapshot::try_from)
                .collect::<Result<_, _>>()?,
        })
    }
}

impl TryFrom<super::WorkspaceSnapshot> for WorkspaceSnapshot {
    type Error = WireError;

    fn try_from(workspace: super::WorkspaceSnapshot) -> WireResult<Self> {
        bounded(workspace.tabs.len(), MAX_SNAPSHOT_TABS, "Tabs")?;
        Ok(Self {
            id: WorkspaceId::from_u64(workspace.id),
            name: workspace.name,
            root_directory: PathBuf::from(workspace.root_directory),
            worktree: workspace.worktree.map(|worktree| WorktreeAssociation {
                parent_workspace_id: WorkspaceId::from_u64(worktree.parent_workspace_id),
                parent_root_directory: PathBuf::from(worktree.parent_root_directory),
                managed: worktree.managed,
            }),
            tabs: workspace
                .tabs
                .into_iter()
                .map(TabSnapshot::try_from)
                .collect::<WireResult<_>>()?,
        })
    }
}

impl TryFrom<&TabSnapshot> for super::TabSnapshot {
    type Error = String;

    fn try_from(tab: &TabSnapshot) -> Result<Self, String> {
        use tab_snapshot::Content;
        let content = match &tab.content {
            TabContentSnapshot::Terminals {
                panes,
                focused_pane,
                focus_history,
                layout,
            } => Content::Terminals(TerminalsTabSnapshot {
                panes: panes
                    .iter()
                    .map(|pane| {
                        Ok::<_, String>(super::PaneSnapshot {
                            id: pane.id.as_u64(),
                            cwd: optional_path_text(pane.cwd.as_ref())?,
                            agent_resume: pane.agent_resume.as_ref().map(super::AgentResume::from),
                        })
                    })
                    .collect::<Result<_, _>>()?,
                focused_pane: focused_pane.as_u64(),
                focus_history: focus_history.iter().map(|pane| pane.as_u64()).collect(),
                layout: Some(layout.into()),
            }),
            TabContentSnapshot::Diff { path } => Content::Diff(PathTabSnapshot {
                path: path.to_string(),
            }),
            TabContentSnapshot::File { path } => Content::File(PathTabSnapshot {
                path: path.to_string(),
            }),
        };
        Ok(Self {
            id: tab.id.as_u64(),
            name: tab.name.clone(),
            content: Some(content),
        })
    }
}

impl TryFrom<super::TabSnapshot> for TabSnapshot {
    type Error = WireError;

    fn try_from(tab: super::TabSnapshot) -> WireResult<Self> {
        use tab_snapshot::Content;
        let content = match member(tab.content)? {
            Content::Terminals(terminals) => {
                bounded(terminals.panes.len(), MAX_SNAPSHOT_PANES, "Panes")?;
                bounded(
                    terminals.focus_history.len(),
                    MAX_SNAPSHOT_PANES,
                    "focus history entries",
                )?;
                TabContentSnapshot::Terminals {
                    panes: terminals
                        .panes
                        .into_iter()
                        .map(|pane| {
                            Ok(PaneSnapshot {
                                id: PaneId::from_u64(pane.id),
                                cwd: pane.cwd.map(PathBuf::from),
                                agent_resume: pane
                                    .agent_resume
                                    .map(AgentResume::try_from)
                                    .transpose()?
                                    .filter(|resume| resume.kind != AgentKind::Other),
                            })
                        })
                        .collect::<WireResult<_>>()?,
                    focused_pane: PaneId::from_u64(terminals.focused_pane),
                    focus_history: terminals
                        .focus_history
                        .into_iter()
                        .map(PaneId::from_u64)
                        .collect(),
                    layout: required(terminals.layout, "layout")?.try_into()?,
                }
            }
            Content::Diff(diff) => TabContentSnapshot::Diff {
                path: relative_path(diff.path)?,
            },
            Content::File(file) => TabContentSnapshot::File {
                path: relative_path(file.path)?,
            },
        };
        Ok(Self {
            id: TabId::from_u64(tab.id),
            name: tab.name,
            content,
        })
    }
}

impl From<&LayoutSnapshot> for super::LayoutSnapshot {
    fn from(layout: &LayoutSnapshot) -> Self {
        use layout_node_snapshot::Node;
        Self {
            root: layout.root,
            nodes: layout
                .nodes
                .iter()
                .map(|node| super::LayoutNodeSnapshot {
                    node: Some(match node {
                        LayoutNodeSnapshot::Pane(pane) => Node::Pane(pane.as_u64()),
                        LayoutNodeSnapshot::Split {
                            direction,
                            ratio,
                            first,
                            second,
                        } => Node::Split(LayoutSplitSnapshot {
                            direction: super::SplitDirection::from(*direction) as i32,
                            ratio: *ratio,
                            first: *first,
                            second: *second,
                        }),
                    }),
                })
                .collect(),
        }
    }
}

impl TryFrom<super::LayoutSnapshot> for LayoutSnapshot {
    type Error = WireError;

    fn try_from(layout: super::LayoutSnapshot) -> WireResult<Self> {
        use layout_node_snapshot::Node;
        bounded(
            layout.nodes.len(),
            MAX_SNAPSHOT_LAYOUT_NODES,
            "layout nodes",
        )?;
        Ok(Self {
            root: layout.root,
            nodes: layout
                .nodes
                .into_iter()
                .map(|node| {
                    Ok(match member(node.node)? {
                        Node::Pane(pane) => LayoutNodeSnapshot::Pane(PaneId::from_u64(pane)),
                        Node::Split(split) => LayoutNodeSnapshot::Split {
                            direction: split_direction(split.direction)?,
                            ratio: split.ratio,
                            first: split.first,
                            second: split.second,
                        },
                    })
                })
                .collect::<WireResult<_>>()?,
        })
    }
}

impl From<SplitDirection> for super::SplitDirection {
    fn from(direction: SplitDirection) -> Self {
        match direction {
            SplitDirection::Horizontal => Self::Horizontal,
            SplitDirection::Vertical => Self::Vertical,
        }
    }
}

pub(crate) fn split_direction(raw: i32) -> WireResult<SplitDirection> {
    Ok(match enum_value(raw, "split direction")? {
        super::SplitDirection::Unspecified => unreachable!("enum_value rejects 0"),
        super::SplitDirection::Horizontal => SplitDirection::Horizontal,
        super::SplitDirection::Vertical => SplitDirection::Vertical,
    })
}

impl From<PaneDirection> for super::PaneDirection {
    fn from(direction: PaneDirection) -> Self {
        match direction {
            PaneDirection::Left => Self::Left,
            PaneDirection::Right => Self::Right,
            PaneDirection::Up => Self::Up,
            PaneDirection::Down => Self::Down,
        }
    }
}

pub(crate) fn pane_direction(raw: i32) -> WireResult<PaneDirection> {
    Ok(match enum_value(raw, "pane direction")? {
        super::PaneDirection::Unspecified => unreachable!("enum_value rejects 0"),
        super::PaneDirection::Left => PaneDirection::Left,
        super::PaneDirection::Right => PaneDirection::Right,
        super::PaneDirection::Up => PaneDirection::Up,
        super::PaneDirection::Down => PaneDirection::Down,
    })
}

impl From<AgentKind> for super::AgentKind {
    fn from(kind: AgentKind) -> Self {
        match kind {
            AgentKind::Claude => Self::Claude,
            AgentKind::Codex => Self::Codex,
            AgentKind::OpenCode => Self::OpenCode,
            AgentKind::Pi => Self::Pi,
            AgentKind::Omp => Self::Omp,
            AgentKind::Antigravity => Self::Antigravity,
            AgentKind::Grok => Self::Grok,
            AgentKind::Cursor => Self::Cursor,
            AgentKind::Copilot => Self::Copilot,
            AgentKind::Kimi => Self::Kimi,
            AgentKind::Other => Self::Other,
        }
    }
}

/// The kind of an agent a command names: one this build does not know makes the
/// command unknown, since nothing here could run it.
pub(crate) fn command_agent_kind(raw: i32) -> WireResult<AgentKind> {
    match enum_value::<super::AgentKind>(raw, "agent kind")? {
        super::AgentKind::Other => Err(WireError::Unknown),
        _ => agent_kind(raw),
    }
}

pub(crate) fn agent_kind(raw: i32) -> WireResult<AgentKind> {
    Ok(match enum_or(raw, "agent kind", super::AgentKind::Other)? {
        super::AgentKind::Unspecified => unreachable!("enum_or rejects 0"),
        super::AgentKind::Claude => AgentKind::Claude,
        super::AgentKind::Codex => AgentKind::Codex,
        super::AgentKind::OpenCode => AgentKind::OpenCode,
        super::AgentKind::Pi => AgentKind::Pi,
        super::AgentKind::Omp => AgentKind::Omp,
        super::AgentKind::Antigravity => AgentKind::Antigravity,
        super::AgentKind::Grok => AgentKind::Grok,
        super::AgentKind::Cursor => AgentKind::Cursor,
        super::AgentKind::Copilot => AgentKind::Copilot,
        super::AgentKind::Kimi => AgentKind::Kimi,
        super::AgentKind::Other => AgentKind::Other,
    })
}

impl From<AgentState> for super::AgentState {
    fn from(state: AgentState) -> Self {
        match state {
            AgentState::Unknown => Self::Unknown,
            AgentState::Idle => Self::Idle,
            AgentState::Working => Self::Working,
            AgentState::Blocked => Self::Blocked,
        }
    }
}

pub(crate) fn agent_state(raw: i32) -> WireResult<AgentState> {
    Ok(
        match enum_or(raw, "agent state", super::AgentState::Unknown)? {
            super::AgentState::Unspecified => unreachable!("enum_or rejects 0"),
            super::AgentState::Unknown => AgentState::Unknown,
            super::AgentState::Idle => AgentState::Idle,
            super::AgentState::Working => AgentState::Working,
            super::AgentState::Blocked => AgentState::Blocked,
        },
    )
}

impl From<&AgentSnapshot> for super::AgentSnapshot {
    fn from(agent: &AgentSnapshot) -> Self {
        Self {
            session_id: agent.session_id.clone(),
            kind: super::AgentKind::from(agent.kind) as i32,
            state: super::AgentState::from(agent.state) as i32,
            blocked_on: agent.blocked_on.clone(),
        }
    }
}

impl TryFrom<super::AgentSnapshot> for AgentSnapshot {
    type Error = WireError;

    fn try_from(agent: super::AgentSnapshot) -> WireResult<Self> {
        Ok(Self {
            session_id: agent.session_id,
            kind: agent_kind(agent.kind)?,
            state: agent_state(agent.state)?,
            blocked_on: agent.blocked_on,
        })
    }
}

impl From<&AgentResume> for super::AgentResume {
    fn from(resume: &AgentResume) -> Self {
        Self {
            kind: super::AgentKind::from(resume.kind) as i32,
            session_id: resume.session_id.clone(),
        }
    }
}

impl TryFrom<super::AgentResume> for AgentResume {
    type Error = WireError;

    fn try_from(resume: super::AgentResume) -> WireResult<Self> {
        Ok(Self {
            kind: agent_kind(resume.kind)?,
            session_id: resume.session_id,
        })
    }
}

impl From<HooksAction> for super::HooksAction {
    fn from(action: HooksAction) -> Self {
        match action {
            HooksAction::Install => Self::Install,
            HooksAction::Uninstall => Self::Uninstall,
            HooksAction::Status => Self::Status,
        }
    }
}

pub(crate) fn hooks_action(raw: i32) -> WireResult<HooksAction> {
    Ok(match enum_value(raw, "hooks action")? {
        super::HooksAction::Unspecified => unreachable!("enum_value rejects 0"),
        super::HooksAction::Install => HooksAction::Install,
        super::HooksAction::Uninstall => HooksAction::Uninstall,
        super::HooksAction::Status => HooksAction::Status,
    })
}

impl TryFrom<&HooksReport> for super::HooksReport {
    type Error = String;

    fn try_from(report: &HooksReport) -> Result<Self, String> {
        let state = match report.state {
            HooksState::Installed => super::HooksState::Installed,
            HooksState::Outdated => super::HooksState::Outdated,
            HooksState::Missing => super::HooksState::Missing,
            HooksState::Unsupported => super::HooksState::Unsupported,
        };
        Ok(Self {
            agent: super::AgentKind::from(report.agent) as i32,
            path: path_text(&report.path)?,
            state: state as i32,
            note: report.note.clone(),
            warning: report.warning.clone(),
        })
    }
}

impl TryFrom<super::HooksReport> for HooksReport {
    type Error = WireError;

    fn try_from(report: super::HooksReport) -> WireResult<Self> {
        let state = match enum_or(report.state, "hooks state", super::HooksState::Unsupported)? {
            super::HooksState::Unspecified => unreachable!("enum_or rejects 0"),
            super::HooksState::Installed => HooksState::Installed,
            super::HooksState::Outdated => HooksState::Outdated,
            super::HooksState::Missing => HooksState::Missing,
            super::HooksState::Unsupported => HooksState::Unsupported,
        };
        Ok(Self {
            agent: agent_kind(report.agent)?,
            path: PathBuf::from(report.path),
            state,
            note: report.note,
            warning: report.warning,
        })
    }
}

impl TryFrom<&AgentInstallation> for super::AgentInstallation {
    type Error = String;

    fn try_from(installation: &AgentInstallation) -> Result<Self, String> {
        Ok(Self {
            kind: super::AgentKind::from(installation.kind) as i32,
            executable: path_text(&installation.executable)?,
        })
    }
}

impl TryFrom<super::AgentInstallation> for AgentInstallation {
    type Error = WireError;

    fn try_from(installation: super::AgentInstallation) -> WireResult<Self> {
        Ok(Self {
            kind: agent_kind(installation.kind)?,
            executable: PathBuf::from(installation.executable),
        })
    }
}

impl From<&GitUpstream> for super::GitUpstream {
    fn from(upstream: &GitUpstream) -> Self {
        Self {
            ahead: upstream.ahead,
            behind: upstream.behind,
        }
    }
}

impl From<super::GitUpstream> for GitUpstream {
    fn from(upstream: super::GitUpstream) -> Self {
        Self {
            ahead: upstream.ahead,
            behind: upstream.behind,
        }
    }
}

impl From<&GitChanges> for super::GitChanges {
    fn from(changes: &GitChanges) -> Self {
        Self {
            entries: changes
                .entries
                .iter()
                .map(|entry| super::GitChangeEntry {
                    path: entry.path.to_string(),
                    old_path: entry.old_path.as_ref().map(ToString::to_string),
                    status: match entry.status {
                        GitChangeStatus::Added => super::GitChangeStatus::Added,
                        GitChangeStatus::Modified => super::GitChangeStatus::Modified,
                        GitChangeStatus::Deleted => super::GitChangeStatus::Deleted,
                        GitChangeStatus::Renamed => super::GitChangeStatus::Renamed,
                        GitChangeStatus::Untracked => super::GitChangeStatus::Untracked,
                        GitChangeStatus::Conflicted => super::GitChangeStatus::Conflicted,
                    } as i32,
                    stat: entry.stat.map(|stat| super::GitDiffStat {
                        added: stat.added,
                        deleted: stat.deleted,
                    }),
                })
                .collect(),
            truncated: changes.truncated,
        }
    }
}

impl TryFrom<super::GitChanges> for GitChanges {
    type Error = WireError;

    fn try_from(changes: super::GitChanges) -> WireResult<Self> {
        Ok(Self {
            entries: changes
                .entries
                .into_iter()
                .map(|entry| {
                    let status = match enum_or(
                        entry.status,
                        "change status",
                        super::GitChangeStatus::Modified,
                    )? {
                        super::GitChangeStatus::Unspecified => unreachable!("enum_or rejects 0"),
                        super::GitChangeStatus::Added => GitChangeStatus::Added,
                        super::GitChangeStatus::Modified => GitChangeStatus::Modified,
                        super::GitChangeStatus::Deleted => GitChangeStatus::Deleted,
                        super::GitChangeStatus::Renamed => GitChangeStatus::Renamed,
                        super::GitChangeStatus::Untracked => GitChangeStatus::Untracked,
                        super::GitChangeStatus::Conflicted => GitChangeStatus::Conflicted,
                    };
                    Ok(GitChangeEntry {
                        path: relative_path(entry.path)?,
                        old_path: entry.old_path.map(relative_path).transpose()?,
                        status,
                        stat: entry.stat.map(|stat| GitDiffStat {
                            added: stat.added,
                            deleted: stat.deleted,
                        }),
                    })
                })
                .collect::<WireResult<_>>()?,
            truncated: changes.truncated,
        })
    }
}

impl From<&FileDiff> for super::FileDiff {
    fn from(diff: &FileDiff) -> Self {
        use file_diff::Content;
        let content = match &diff.content {
            FileDiffContent::Text { hunks } => Content::Text(DiffHunks {
                hunks: hunks
                    .iter()
                    .map(|hunk| super::DiffHunk {
                        old_start: hunk.old_start,
                        old_lines: hunk.old_lines,
                        new_start: hunk.new_start,
                        new_lines: hunk.new_lines,
                        lines: hunk
                            .lines
                            .iter()
                            .map(|line| super::DiffLine {
                                kind: match line.kind {
                                    DiffLineKind::Context => super::DiffLineKind::Context,
                                    DiffLineKind::Added => super::DiffLineKind::Added,
                                    DiffLineKind::Removed => super::DiffLineKind::Removed,
                                } as i32,
                                old_number: line.old_number,
                                new_number: line.new_number,
                                text: line.text.clone(),
                            })
                            .collect(),
                    })
                    .collect(),
            }),
            FileDiffContent::Binary => Content::Binary(Empty {}),
            FileDiffContent::TooLarge { bytes } => Content::TooLarge(*bytes),
        };
        Self {
            path: diff.path.to_string(),
            content: Some(content),
        }
    }
}

impl TryFrom<super::FileDiff> for FileDiff {
    type Error = WireError;

    fn try_from(diff: super::FileDiff) -> WireResult<Self> {
        use file_diff::Content;
        let content = match member(diff.content)? {
            Content::Text(text) => FileDiffContent::Text {
                hunks: text
                    .hunks
                    .into_iter()
                    .map(|hunk| {
                        Ok(DiffHunk {
                            old_start: hunk.old_start,
                            old_lines: hunk.old_lines,
                            new_start: hunk.new_start,
                            new_lines: hunk.new_lines,
                            lines: hunk
                                .lines
                                .into_iter()
                                .map(|line| {
                                    let kind = match enum_or(
                                        line.kind,
                                        "diff line kind",
                                        super::DiffLineKind::Context,
                                    )? {
                                        super::DiffLineKind::Unspecified => {
                                            unreachable!("enum_or rejects 0")
                                        }
                                        super::DiffLineKind::Context => DiffLineKind::Context,
                                        super::DiffLineKind::Added => DiffLineKind::Added,
                                        super::DiffLineKind::Removed => DiffLineKind::Removed,
                                    };
                                    Ok(DiffLine {
                                        kind,
                                        old_number: line.old_number,
                                        new_number: line.new_number,
                                        text: line.text,
                                    })
                                })
                                .collect::<WireResult<_>>()?,
                        })
                    })
                    .collect::<WireResult<_>>()?,
            },
            Content::Binary(_) => FileDiffContent::Binary,
            Content::TooLarge(bytes) => FileDiffContent::TooLarge { bytes },
        };
        Ok(Self {
            path: relative_path(diff.path)?,
            content,
        })
    }
}

impl From<&DirectoryListing> for super::DirectoryListing {
    fn from(listing: &DirectoryListing) -> Self {
        Self {
            entries: listing
                .entries
                .iter()
                .map(|entry| super::DirectoryEntry {
                    name: entry.name.clone(),
                    kind: match entry.kind {
                        FileKind::Directory => super::FileKind::Directory,
                        FileKind::File => super::FileKind::File,
                    } as i32,
                    ignored: entry.ignored,
                })
                .collect(),
            truncated: listing.truncated,
        }
    }
}

impl TryFrom<super::DirectoryListing> for DirectoryListing {
    type Error = WireError;

    fn try_from(listing: super::DirectoryListing) -> WireResult<Self> {
        Ok(Self {
            entries: listing
                .entries
                .into_iter()
                .map(|entry| {
                    let kind = match enum_or(entry.kind, "file kind", super::FileKind::File)? {
                        super::FileKind::Unspecified => unreachable!("enum_or rejects 0"),
                        super::FileKind::Directory => FileKind::Directory,
                        super::FileKind::File => FileKind::File,
                    };
                    Ok(DirectoryEntry {
                        name: entry.name,
                        kind,
                        ignored: entry.ignored,
                    })
                })
                .collect::<WireResult<_>>()?,
            truncated: listing.truncated,
        })
    }
}

impl From<&FileContent> for super::FileContent {
    fn from(content: &FileContent) -> Self {
        use file_content::Content;
        Self {
            content: Some(match content {
                FileContent::Text { text } => Content::Text(text.clone()),
                FileContent::Binary => Content::Binary(Empty {}),
                FileContent::TooLarge { bytes } => Content::TooLarge(*bytes),
            }),
        }
    }
}

impl TryFrom<super::FileContent> for FileContent {
    type Error = WireError;

    fn try_from(content: super::FileContent) -> WireResult<Self> {
        use file_content::Content;
        Ok(match member(content.content)? {
            Content::Text(text) => Self::Text { text },
            Content::Binary(_) => Self::Binary,
            Content::TooLarge(bytes) => Self::TooLarge { bytes },
        })
    }
}
