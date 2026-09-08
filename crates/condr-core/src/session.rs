mod layout;
mod pane;
mod snapshot;
mod workspace;

use crate::snapshot::{
    LayoutNodeSnapshot, LayoutSnapshot, MAX_SNAPSHOT_LAYOUT_DEPTH, MAX_SNAPSHOT_LAYOUT_NODES,
    MAX_SNAPSHOT_PANES, MAX_SNAPSHOT_TABS, MAX_SNAPSHOT_WORKSPACES, PaneSnapshot, SNAPSHOT_VERSION,
    SessionSnapshot, SnapshotError, TabSnapshot, WorkspaceSnapshot,
};
use layout::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
const MAX_STABLE_ID: u64 = u64::MAX / 2;
const MAX_NEXT_STABLE_ID_EXCLUSIVE: u64 = MAX_STABLE_ID + 1;

fn reserve_ids(count: u64) -> Option<u64> {
    debug_assert!(count > 0);
    NEXT_ID
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
            next.checked_add(count)
                .filter(|end| *end <= MAX_NEXT_STABLE_ID_EXCLUSIVE)
        })
        .ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkspaceId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TabId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PaneId(u64);

impl WorkspaceId {
    /// The inverse of [`Self::as_u64`], for ids that arrive as CLI arguments.
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl TabId {
    /// The inverse of [`Self::as_u64`], for ids that arrive as CLI arguments.
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl PaneId {
    /// The inverse of [`Self::as_u64`], for ids that arrive as text (`CONDR_PANE_ID`).
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }

    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PaneDirection {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PaneLayout {
    Pane(PaneId),
    Split {
        direction: SplitDirection,
        ratio: f32,
        first: Box<PaneLayout>,
        second: Box<PaneLayout>,
    },
}

#[derive(Clone, Debug, Default)]
pub struct Session {
    workspaces: Vec<Workspace>,
    active_workspace: Option<WorkspaceId>,
}

#[derive(Clone, Debug)]
pub struct Workspace {
    id: WorkspaceId,
    name: String,
    root_directory: PathBuf,
    worktree: Option<WorktreeAssociation>,
    tabs: Vec<Tab>,
    active_tab: TabId,
}

#[derive(Clone, Debug)]
pub struct Tab {
    id: TabId,
    name: String,
    panes: Vec<Pane>,
    focused_pane: PaneId,
    focus_history: Vec<PaneId>,
    layout: PaneLayout,
    zoomed_pane: Option<PaneId>,
}

#[derive(Clone, Debug)]
pub struct Pane {
    id: PaneId,
    cwd: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeAssociation {
    parent_workspace_id: WorkspaceId,
    parent_root_directory: PathBuf,
    managed: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CloseOutcome {
    panes: Vec<PaneId>,
    tabs: Vec<TabId>,
    workspaces: Vec<WorkspaceId>,
}

impl Session {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.workspaces.is_empty()
    }

    pub fn workspaces(&self) -> &[Workspace] {
        &self.workspaces
    }

    pub fn active_workspace_id(&self) -> Option<WorkspaceId> {
        self.active_workspace
    }

    pub fn active_workspace(&self) -> Option<&Workspace> {
        let id = self.active_workspace?;
        self.workspaces.iter().find(|workspace| workspace.id == id)
    }

    pub fn workspace(&self, workspace_id: WorkspaceId) -> Option<&Workspace> {
        self.workspaces
            .iter()
            .find(|workspace| workspace.id == workspace_id)
    }

    pub fn tab(&self, tab_id: TabId) -> Option<&Tab> {
        self.workspaces
            .iter()
            .flat_map(Workspace::tabs)
            .find(|tab| tab.id == tab_id)
    }

    pub fn pane(&self, pane_id: PaneId) -> Option<&Pane> {
        self.workspaces
            .iter()
            .flat_map(Workspace::tabs)
            .flat_map(Tab::panes)
            .find(|pane| pane.id == pane_id)
    }

    pub fn workspace_for_pane(&self, pane_id: PaneId) -> Option<&Workspace> {
        self.workspaces.iter().find(|workspace| {
            workspace
                .tabs
                .iter()
                .flat_map(Tab::panes)
                .any(|pane| pane.id == pane_id)
        })
    }

    pub fn workspace_by_root(&self, root_directory: &Path) -> Option<&Workspace> {
        self.workspaces
            .iter()
            .find(|workspace| workspace.root_directory == root_directory)
    }

    fn find_pane(&self, pane_id: PaneId) -> Option<(usize, usize, usize)> {
        // ponytail: collections are small; add ID indexes only if profiling shows lookup cost.
        for (workspace_ix, workspace) in self.workspaces.iter().enumerate() {
            for (tab_ix, tab) in workspace.tabs.iter().enumerate() {
                if let Some(pane_ix) = tab.panes.iter().position(|pane| pane.id == pane_id) {
                    return Some((workspace_ix, tab_ix, pane_ix));
                }
            }
        }
        None
    }

    fn tab_count(&self) -> usize {
        self.workspaces
            .iter()
            .map(|workspace| workspace.tabs.len())
            .sum()
    }

    fn pane_count(&self) -> usize {
        self.workspaces
            .iter()
            .flat_map(|workspace| &workspace.tabs)
            .map(|tab| tab.panes.len())
            .sum()
    }
}

impl CloseOutcome {
    pub fn panes(&self) -> &[PaneId] {
        &self.panes
    }

    pub fn tabs(&self) -> &[TabId] {
        &self.tabs
    }

    pub fn workspaces(&self) -> &[WorkspaceId] {
        &self.workspaces
    }
}

impl Workspace {
    pub fn id(&self) -> WorkspaceId {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn root_directory(&self) -> &Path {
        &self.root_directory
    }

    pub fn worktree(&self) -> Option<&WorktreeAssociation> {
        self.worktree.as_ref()
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn active_tab(&self) -> &Tab {
        self.tabs
            .iter()
            .find(|tab| tab.id == self.active_tab)
            .expect("active tab belongs to workspace")
    }
}

impl Tab {
    pub fn id(&self) -> TabId {
        self.id
    }

    /// The user-provided name, or empty for an unnamed Tab. Its display number
    /// comes from its current position in the Workspace, not its stable ID.
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn panes(&self) -> &[Pane] {
        &self.panes
    }

    pub fn focused_pane(&self) -> &Pane {
        self.panes
            .iter()
            .find(|pane| pane.id == self.focused_pane)
            .expect("focused pane belongs to tab")
    }

    pub fn focus_history(&self) -> &[PaneId] {
        &self.focus_history
    }

    pub fn layout(&self) -> &PaneLayout {
        &self.layout
    }

    /// Where every Pane sits, in layout order.
    pub fn pane_rects(&self) -> Vec<PaneRect> {
        let mut rects = Vec::with_capacity(self.panes.len());
        collect_pane_rects(&self.layout, 0.0, 0.0, 1.0, 1.0, &mut rects);
        rects
    }

    /// The Pane that focus, swap and resize in `direction` would reach.
    pub fn neighbor(&self, pane_id: PaneId, direction: PaneDirection) -> Option<PaneId> {
        neighbor_pane_id(&self.layout, pane_id, direction)
    }

    pub fn zoomed_pane_id(&self) -> Option<PaneId> {
        self.zoomed_pane
    }
}

impl Pane {
    pub fn id(&self) -> PaneId {
        self.id
    }

    pub fn cwd(&self) -> Option<&Path> {
        self.cwd.as_deref()
    }
}

impl WorktreeAssociation {
    pub fn parent_workspace_id(&self) -> WorkspaceId {
        self.parent_workspace_id
    }

    pub fn parent_root_directory(&self) -> &Path {
        &self.parent_root_directory
    }

    pub fn is_managed(&self) -> bool {
        self.managed
    }
}

/// A Pane's place in its Tab, as fractions of the Tab area (0 to 1). An edge at 0 or 1
/// touches the Tab's border.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaneRect {
    pub id: PaneId,
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}
