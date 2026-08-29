use std::{fmt, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{PaneId, PaneLayout, TabId, WorkspaceId, WorktreeAssociation};

pub(crate) const SNAPSHOT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub(crate) version: u32,
    pub(crate) workspaces: Vec<WorkspaceSnapshot>,
    pub(crate) active_workspace: Option<WorkspaceId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct WorkspaceSnapshot {
    pub(crate) id: WorkspaceId,
    pub(crate) name: String,
    pub(crate) root_directory: PathBuf,
    pub(crate) worktree: Option<WorktreeAssociation>,
    pub(crate) tabs: Vec<TabSnapshot>,
    pub(crate) active_tab: TabId,
    pub(crate) next_tab_number: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct TabSnapshot {
    pub(crate) id: TabId,
    pub(crate) name: String,
    pub(crate) panes: Vec<PaneSnapshot>,
    pub(crate) focused_pane: PaneId,
    pub(crate) focus_history: Vec<PaneId>,
    pub(crate) layout: PaneLayout,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct PaneSnapshot {
    pub(crate) id: PaneId,
    pub(crate) cwd: Option<PathBuf>,
}

impl SessionSnapshot {
    pub fn to_bytes(&self) -> bincode::Result<Vec<u8>> {
        bincode::serialize(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> bincode::Result<Self> {
        bincode::deserialize(bytes)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    UnsupportedVersion(u32),
    Invalid(&'static str),
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => {
                write!(formatter, "unsupported Session Snapshot version {version}")
            }
            Self::Invalid(reason) => write!(formatter, "invalid Session Snapshot: {reason}"),
        }
    }
}

impl std::error::Error for SnapshotError {}
