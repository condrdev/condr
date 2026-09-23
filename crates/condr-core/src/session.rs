mod layout;
mod pane;
mod snapshot;
mod workspace;

use crate::snapshot::{
    LayoutNodeSnapshot, LayoutSnapshot, MAX_SNAPSHOT_LAYOUT_DEPTH, MAX_SNAPSHOT_LAYOUT_NODES,
    MAX_SNAPSHOT_PANES, MAX_SNAPSHOT_TABS, MAX_SNAPSHOT_WORKSPACES, PaneSnapshot, SessionSnapshot,
    SnapshotError, TabSnapshot, WorkspaceSnapshot,
};
use layout::*;
use relative_path::{RelativePath, RelativePathBuf};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
pub use workspace::{DIFF_TAB_NAME, FILE_TAB_NAME};

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
/// The Session structure every client shares. Which Workspace and Tab a client shows is
/// that client's own view state and lives outside the Session (ADR 0021).
pub struct Session {
    workspaces: Vec<Workspace>,
}

#[derive(Clone, Debug)]
pub struct Workspace {
    id: WorkspaceId,
    name: String,
    root_directory: PathBuf,
    worktree: Option<WorktreeAssociation>,
    tabs: Vec<Tab>,
}

#[derive(Clone, Debug)]
pub struct Tab {
    id: TabId,
    name: String,
    content: TabContent,
}

/// What a Tab shows: a layout of terminal Panes, or a viewer with no Panes at all
/// (ADR 0017, ADR 0018). Pane commands only ever reach terminal Tabs, since a viewer has
/// none. A Workspace has at most one viewer Tab of each kind.
#[derive(Clone, Debug)]
pub enum TabContent {
    Terminals(TerminalLayout),
    Diff(DiffView),
    File(FileView),
}

impl TabContent {
    /// Whether two contents are the same viewer kind; terminal layouts never are.
    fn same_viewer_kind(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Diff(_), Self::Diff(_)) | (Self::File(_), Self::File(_))
        )
    }
}

#[derive(Clone, Debug)]
pub struct TerminalLayout {
    panes: Vec<Pane>,
    focused_pane: PaneId,
    focus_history: Vec<PaneId>,
    layout: PaneLayout,
    zoomed_pane: Option<PaneId>,
}

/// One file's working-tree diff against `HEAD`, named relative to the Workspace root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffView {
    path: RelativePathBuf,
}

/// One file's content as it is on disk (ADR 0018), named relative to the Workspace root.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileView {
    path: RelativePathBuf,
}

impl FileView {
    pub fn path(&self) -> &RelativePath {
        &self.path
    }
}

#[derive(Clone, Debug)]
pub struct Pane {
    id: PaneId,
    cwd: Option<PathBuf>,
    agent_resume: Option<crate::AgentResume>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeAssociation {
    pub(crate) parent_workspace_id: WorkspaceId,
    pub(crate) parent_root_directory: PathBuf,
    pub(crate) managed: bool,
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

    /// The Workspace's Diff Tab, if it has one; at most one exists (ADR 0017).
    pub fn diff_tab(&self, workspace_id: WorkspaceId) -> Option<&Tab> {
        self.workspace(workspace_id)?
            .tabs
            .iter()
            .find(|tab| tab.diff().is_some())
    }

    /// The Workspace's Preview Tab, if it has one; at most one exists (ADR 0018).
    pub fn file_tab(&self, workspace_id: WorkspaceId) -> Option<&Tab> {
        self.workspace(workspace_id)?
            .tabs
            .iter()
            .find(|tab| tab.file().is_some())
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
                if let Some(pane_ix) = tab.panes().iter().position(|pane| pane.id == pane_id) {
                    return Some((workspace_ix, tab_ix, pane_ix));
                }
            }
        }
        None
    }

    /// The terminal layout of the Tab `find_pane` located: a Pane is always in one.
    fn terminal_layout_mut(&mut self, workspace_ix: usize, tab_ix: usize) -> &mut TerminalLayout {
        self.workspaces[workspace_ix].tabs[tab_ix]
            .terminals_mut()
            .expect("a Pane lives in a terminal Tab")
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
            .map(|tab| tab.panes().len())
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

    pub fn tab(&self, tab_id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id == tab_id)
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

    pub fn content(&self) -> &TabContent {
        &self.content
    }

    pub fn terminals(&self) -> Option<&TerminalLayout> {
        match &self.content {
            TabContent::Terminals(terminals) => Some(terminals),
            TabContent::Diff(_) | TabContent::File(_) => None,
        }
    }

    fn terminals_mut(&mut self) -> Option<&mut TerminalLayout> {
        match &mut self.content {
            TabContent::Terminals(terminals) => Some(terminals),
            TabContent::Diff(_) | TabContent::File(_) => None,
        }
    }

    pub fn diff(&self) -> Option<&DiffView> {
        match &self.content {
            TabContent::Diff(diff) => Some(diff),
            TabContent::Terminals(_) | TabContent::File(_) => None,
        }
    }

    pub fn file(&self) -> Option<&FileView> {
        match &self.content {
            TabContent::File(file) => Some(file),
            TabContent::Terminals(_) | TabContent::Diff(_) => None,
        }
    }

    /// Empty for a viewer Tab.
    pub fn panes(&self) -> &[Pane] {
        self.terminals()
            .map_or(&[], |terminals| terminals.panes.as_slice())
    }

    /// `None` for a viewer Tab; a terminal Tab always has one.
    pub fn focused_pane(&self) -> Option<&Pane> {
        let terminals = self.terminals()?;
        Some(
            terminals
                .panes
                .iter()
                .find(|pane| pane.id == terminals.focused_pane)
                .expect("focused pane belongs to tab"),
        )
    }

    pub fn focus_history(&self) -> &[PaneId] {
        self.terminals()
            .map_or(&[], |terminals| terminals.focus_history.as_slice())
    }

    pub fn layout(&self) -> Option<&PaneLayout> {
        self.terminals().map(|terminals| &terminals.layout)
    }

    /// Where every Pane sits, in layout order; empty for a viewer Tab.
    pub fn pane_rects(&self) -> Vec<PaneRect> {
        let Some(terminals) = self.terminals() else {
            return Vec::new();
        };
        let mut rects = Vec::with_capacity(terminals.panes.len());
        collect_pane_rects(&terminals.layout, 0.0, 0.0, 1.0, 1.0, &mut rects);
        rects
    }

    /// The Pane that focus, swap and resize in `direction` would reach.
    pub fn neighbor(&self, pane_id: PaneId, direction: PaneDirection) -> Option<PaneId> {
        neighbor_pane_id(&self.terminals()?.layout, pane_id, direction)
    }

    pub fn zoomed_pane_id(&self) -> Option<PaneId> {
        self.terminals()?.zoomed_pane
    }
}

impl TerminalLayout {
    fn single(pane: Pane) -> Self {
        let pane_id = pane.id;
        Self {
            panes: vec![pane],
            focused_pane: pane_id,
            focus_history: Vec::new(),
            layout: PaneLayout::Pane(pane_id),
            zoomed_pane: None,
        }
    }
}

impl DiffView {
    /// Relative to the Workspace root.
    pub fn path(&self) -> &RelativePath {
        &self.path
    }
}

/// A Diff Tab names its file relative to the Workspace root: non-empty and made of plain
/// names only, so it cannot reach outside the working tree. A [`RelativePath`] is
/// `/`-separated on every platform; the Server turns it into a native path under the root.
pub fn valid_diff_path(path: &RelativePath) -> bool {
    !path.as_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, relative_path::Component::Normal(_)))
}

impl Pane {
    pub fn agent_resume(&self) -> Option<&crate::AgentResume> {
        self.agent_resume.as_ref()
    }

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
