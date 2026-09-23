use std::{
    fmt,
    path::{Path, PathBuf},
};

use prost::Message as _;

use crate::{PaneId, PaneLayout, SplitDirection, TabId, WorkspaceId, WorktreeAssociation};

pub(crate) const MAX_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_SNAPSHOT_WORKSPACES: usize = 64;
pub(crate) const MAX_SNAPSHOT_TABS: usize = 256;
pub(crate) const MAX_SNAPSHOT_PANES: usize = 1_024;
pub(crate) const MAX_SNAPSHOT_LAYOUT_DEPTH: usize = 64;
pub(crate) const MAX_SNAPSHOT_LAYOUT_NODES: usize = MAX_SNAPSHOT_PANES * 2 - 1;

#[derive(Clone, Debug, PartialEq)]
pub struct SessionSnapshot {
    pub(crate) workspaces: Vec<WorkspaceSnapshot>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct WorkspaceSnapshot {
    pub(crate) id: WorkspaceId,
    pub(crate) name: String,
    pub(crate) root_directory: PathBuf,
    pub(crate) worktree: Option<WorktreeAssociation>,
    pub(crate) tabs: Vec<TabSnapshot>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct TabSnapshot {
    pub(crate) id: TabId,
    pub(crate) name: String,
    pub(crate) content: TabContentSnapshot,
}

/// A Tab's content (ADR 0017): its terminal layout, or the file a Diff Tab shows.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum TabContentSnapshot {
    Terminals {
        panes: Vec<PaneSnapshot>,
        focused_pane: PaneId,
        focus_history: Vec<PaneId>,
        layout: LayoutSnapshot,
    },
    Diff {
        path: relative_path::RelativePathBuf,
    },
    /// The file a Preview Tab shows (ADR 0018).
    File {
        path: relative_path::RelativePathBuf,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PaneSnapshot {
    pub(crate) id: PaneId,
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) agent_resume: Option<crate::AgentResume>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LayoutSnapshot {
    pub(crate) root: u32,
    pub(crate) nodes: Vec<LayoutNodeSnapshot>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LayoutNodeSnapshot {
    Pane(PaneId),
    Split {
        direction: SplitDirection,
        ratio: f32,
        first: u32,
        second: u32,
    },
}

impl LayoutSnapshot {
    pub(crate) fn from_layout(layout: &PaneLayout) -> Self {
        let mut pending = vec![layout];
        let mut nodes = Vec::new();
        let mut cursor = 0;

        while let Some(layout) = pending.get(cursor).copied() {
            let node = match layout {
                PaneLayout::Pane(pane_id) => LayoutNodeSnapshot::Pane(*pane_id),
                PaneLayout::Split {
                    direction,
                    ratio,
                    first,
                    second,
                } => {
                    let first_index = u32::try_from(pending.len())
                        .expect("a durable layout has fewer than u32::MAX nodes");
                    pending.push(first);
                    let second_index = u32::try_from(pending.len())
                        .expect("a durable layout has fewer than u32::MAX nodes");
                    pending.push(second);
                    LayoutNodeSnapshot::Split {
                        direction: *direction,
                        ratio: *ratio,
                        first: first_index,
                        second: second_index,
                    }
                }
            };
            nodes.push(node);
            cursor += 1;
        }

        Self { root: 0, nodes }
    }
}

impl SessionSnapshot {
    pub fn root_paths(&self) -> impl Iterator<Item = &Path> {
        self.workspaces.iter().flat_map(|workspace| {
            std::iter::once(workspace.root_directory.as_path()).chain(
                workspace
                    .worktree
                    .iter()
                    .map(|worktree| worktree.parent_root_directory()),
            )
        })
    }

    /// The Snapshot as written to disk: the same protobuf message the wire carries
    /// (ADR 0028). Fails for a path that is not UTF-8 or a Snapshot over the size limit.
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let wire = crate::protocol::pb::SessionSnapshot::try_from(self)?;
        let length = wire.encoded_len();
        if length > MAX_SNAPSHOT_BYTES {
            return Err(format!(
                "Session Snapshot is {length} bytes; limit is {MAX_SNAPSHOT_BYTES} bytes"
            ));
        }
        Ok(wire.encode_to_vec())
    }

    /// Reads a Snapshot back. One written by a newer Server loses what this build does
    /// not know, and one with a Tab kind it does not know is refused whole.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(format!(
                "Session Snapshot is {} bytes; limit is {MAX_SNAPSHOT_BYTES} bytes",
                bytes.len()
            ));
        }
        let wire = crate::protocol::pb::SessionSnapshot::decode(bytes)
            .map_err(|error| error.to_string())?;
        Self::try_from(wire).map_err(|error| error.to_string())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SnapshotError {
    Invalid(&'static str),
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(reason) => write!(formatter, "invalid Session Snapshot: {reason}"),
        }
    }
}

impl std::error::Error for SnapshotError {}
