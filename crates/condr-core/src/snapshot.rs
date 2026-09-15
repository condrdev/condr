use std::{
    fmt,
    marker::PhantomData,
    path::{Path, PathBuf},
};

use bincode::Options as _;
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{self, SeqAccess, Visitor},
};

use crate::{PaneId, PaneLayout, SplitDirection, TabId, WorkspaceId, WorktreeAssociation};

pub(crate) const SNAPSHOT_VERSION: u32 = 1;
pub(crate) const MAX_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
pub(crate) const MAX_SNAPSHOT_WORKSPACES: usize = 64;
pub(crate) const MAX_SNAPSHOT_TABS: usize = 256;
pub(crate) const MAX_SNAPSHOT_PANES: usize = 1_024;
pub(crate) const MAX_SNAPSHOT_LAYOUT_DEPTH: usize = 64;
pub(crate) const MAX_SNAPSHOT_LAYOUT_NODES: usize = MAX_SNAPSHOT_PANES * 2 - 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub(crate) version: u32,
    #[serde(deserialize_with = "deserialize_workspaces")]
    pub(crate) workspaces: Vec<WorkspaceSnapshot>,
    pub(crate) active_workspace: Option<WorkspaceId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct WorkspaceSnapshot {
    pub(crate) id: WorkspaceId,
    pub(crate) name: String,
    pub(crate) root_directory: PathBuf,
    pub(crate) worktree: Option<WorktreeAssociation>,
    #[serde(deserialize_with = "deserialize_tabs")]
    pub(crate) tabs: Vec<TabSnapshot>,
    pub(crate) active_tab: TabId,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct TabSnapshot {
    pub(crate) id: TabId,
    pub(crate) name: String,
    pub(crate) content: TabContentSnapshot,
}

/// A Tab's content (ADR 0017): its terminal layout, or the file a Diff Tab shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum TabContentSnapshot {
    Terminals {
        #[serde(deserialize_with = "deserialize_panes")]
        panes: Vec<PaneSnapshot>,
        focused_pane: PaneId,
        #[serde(deserialize_with = "deserialize_focus_history")]
        focus_history: Vec<PaneId>,
        layout: LayoutSnapshot,
    },
    Diff {
        path: PathBuf,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct PaneSnapshot {
    pub(crate) id: PaneId,
    pub(crate) cwd: Option<PathBuf>,
    pub(crate) agent_resume: Option<crate::AgentResume>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct LayoutSnapshot {
    pub(crate) root: u32,
    #[serde(deserialize_with = "deserialize_layout_nodes")]
    pub(crate) nodes: Vec<LayoutNodeSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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

    pub fn to_bytes(&self) -> bincode::Result<Vec<u8>> {
        bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .with_limit(MAX_SNAPSHOT_BYTES as u64)
            .serialize(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> bincode::Result<Self> {
        if bytes.len() > MAX_SNAPSHOT_BYTES {
            return Err(Box::new(bincode::ErrorKind::SizeLimit));
        }
        bincode::DefaultOptions::new()
            .with_fixint_encoding()
            .with_limit(MAX_SNAPSHOT_BYTES as u64)
            .reject_trailing_bytes()
            .deserialize(bytes)
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

fn deserialize_workspaces<'de, D>(deserializer: D) -> Result<Vec<WorkspaceSnapshot>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_vec(
        deserializer,
        MAX_SNAPSHOT_WORKSPACES,
        "Session Snapshot Workspaces",
    )
}

fn deserialize_tabs<'de, D>(deserializer: D) -> Result<Vec<TabSnapshot>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_vec(deserializer, MAX_SNAPSHOT_TABS, "Session Snapshot Tabs")
}

fn deserialize_panes<'de, D>(deserializer: D) -> Result<Vec<PaneSnapshot>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_vec(deserializer, MAX_SNAPSHOT_PANES, "Session Snapshot Panes")
}

fn deserialize_focus_history<'de, D>(deserializer: D) -> Result<Vec<PaneId>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_vec(
        deserializer,
        MAX_SNAPSHOT_PANES,
        "Session Snapshot focus history",
    )
}

fn deserialize_layout_nodes<'de, D>(deserializer: D) -> Result<Vec<LayoutNodeSnapshot>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_bounded_vec(
        deserializer,
        MAX_SNAPSHOT_LAYOUT_NODES,
        "Session Snapshot layout nodes",
    )
}

fn deserialize_bounded_vec<'de, D, T>(
    deserializer: D,
    limit: usize,
    description: &'static str,
) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    deserializer.deserialize_seq(BoundedVecVisitor {
        limit,
        description,
        item: PhantomData,
    })
}

struct BoundedVecVisitor<T> {
    limit: usize,
    description: &'static str,
    item: PhantomData<T>,
}

impl<'de, T> Visitor<'de> for BoundedVecVisitor<T>
where
    T: Deserialize<'de>,
{
    type Value = Vec<T>;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} containing at most {} entries",
            self.description, self.limit
        )
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let capacity = sequence.size_hint().unwrap_or(0);
        if capacity > self.limit {
            return Err(de::Error::invalid_length(capacity, &self));
        }
        let mut values = Vec::with_capacity(capacity);
        while let Some(value) = sequence.next_element()? {
            if values.len() == self.limit {
                return Err(de::Error::invalid_length(values.len() + 1, &self));
            }
            values.push(value);
        }
        Ok(values)
    }
}
