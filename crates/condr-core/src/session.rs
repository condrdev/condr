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

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
const MAX_STABLE_ID: u64 = u64::MAX / 2;
const MAX_NEXT_STABLE_ID_EXCLUSIVE: u64 = MAX_STABLE_ID + 1;

fn reserve_ids(count: u64) -> Option<u64> {
    debug_assert!(count > 0);
    NEXT_ID
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
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
    layout: TabLayout,
}

/// The viewer a viewer Pane shows (ADR 0078), which `ShowViewer` creates or retargets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Viewer {
    /// `path`'s diff (ADR 0017).
    Diff { path: RelativePathBuf },
    /// `path`'s content (ADR 0018).
    File { path: RelativePathBuf },
    /// The commit graph (ADR 0079).
    GitGraph,
}

/// A viewer's kind, of which a Workspace has at most one Pane (ADR 0078).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ViewerKind {
    Diff,
    File,
    GitGraph,
}

impl Viewer {
    pub fn kind(&self) -> ViewerKind {
        match self {
            Self::Diff { .. } => ViewerKind::Diff,
            Self::File { .. } => ViewerKind::File,
            Self::GitGraph => ViewerKind::GitGraph,
        }
    }

    /// The file it shows, relative to the Workspace root.
    pub fn path(&self) -> Option<&RelativePath> {
        match self {
            Self::Diff { path } | Self::File { path } => Some(path),
            Self::GitGraph => None,
        }
    }

    fn valid(&self) -> bool {
        self.path().is_none_or(valid_diff_path)
    }
}

impl ViewerKind {
    /// What a Tab is called while this viewer is its focused Pane and nobody named it; the
    /// same in every language (ADR 0078).
    pub fn name(self) -> &'static str {
        match self {
            Self::Diff => "Diff",
            Self::File => "Preview",
            Self::GitGraph => "Git Graph",
        }
    }
}

/// A Tab's Panes and how they are laid out.
#[derive(Clone, Debug)]
struct TabLayout {
    panes: Vec<Pane>,
    focused_pane: PaneId,
    focus_history: Vec<PaneId>,
    layout: PaneLayout,
    zoomed_pane: Option<PaneId>,
}

#[derive(Clone, Debug)]
pub struct Pane {
    id: PaneId,
    content: PaneContent,
}

/// What a Pane holds (ADR 0078).
#[derive(Clone, Debug, PartialEq)]
pub enum PaneContent {
    /// A shell, started where `cwd` names, and the Agent conversation to resume in it.
    Terminal {
        cwd: Option<PathBuf>,
        agent_resume: Option<crate::AgentResume>,
    },
    Viewer(Viewer),
    /// A kind a newer peer added that this build does not know: it keeps its place and
    /// shows nothing.
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeAssociation {
    pub(crate) parent_workspace_id: WorkspaceId,
    pub(crate) parent_root_directory: PathBuf,
    pub(crate) managed: bool,
    /// The parent's branch the worktree's branch was made from (ADR 0034); `None` when
    /// Condr did not create the branch.
    pub(crate) base_branch: Option<String>,
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

    /// The Workspace's viewer Pane of `kind`, and the Tab it is in; at most one exists
    /// (ADR 0078).
    pub fn viewer_pane(
        &self,
        workspace_id: WorkspaceId,
        kind: ViewerKind,
    ) -> Option<(&Tab, &Pane)> {
        self.workspace(workspace_id)?.tabs.iter().find_map(|tab| {
            tab.panes()
                .iter()
                .find(|pane| pane.viewer().is_some_and(|viewer| viewer.kind() == kind))
                .map(|pane| (tab, pane))
        })
    }

    pub fn tab_for_pane(&self, pane_id: PaneId) -> Option<&Tab> {
        self.workspaces
            .iter()
            .flat_map(Workspace::tabs)
            .find(|tab| tab.panes().iter().any(|pane| pane.id == pane_id))
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

    /// The layout of the Tab `find_pane` located.
    fn tab_layout_mut(&mut self, workspace_ix: usize, tab_ix: usize) -> &mut TabLayout {
        &mut self.workspaces[workspace_ix].tabs[tab_ix].layout
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

    pub fn panes(&self) -> &[Pane] {
        &self.layout.panes
    }

    /// Always `Some`: a Tab has at least one Pane, and one has the focus.
    pub fn focused_pane(&self) -> Option<&Pane> {
        self.layout
            .panes
            .iter()
            .find(|pane| pane.id == self.layout.focused_pane)
    }

    pub fn focus_history(&self) -> &[PaneId] {
        &self.layout.focus_history
    }

    /// Always `Some`.
    pub fn layout(&self) -> Option<&PaneLayout> {
        Some(&self.layout.layout)
    }

    /// Where every Pane sits, in layout order.
    pub fn pane_rects(&self) -> Vec<PaneRect> {
        let mut rects = Vec::with_capacity(self.layout.panes.len());
        collect_pane_rects(&self.layout.layout, 0.0, 0.0, 1.0, 1.0, &mut rects);
        rects
    }

    /// The Pane that focus, swap and resize in `direction` would reach.
    pub fn neighbor(&self, pane_id: PaneId, direction: PaneDirection) -> Option<PaneId> {
        neighbor_pane_id(&self.layout.layout, pane_id, direction)
    }

    pub fn zoomed_pane_id(&self) -> Option<PaneId> {
        self.layout.zoomed_pane
    }
}

impl TabLayout {
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

/// A viewer names its file relative to the Workspace root: non-empty and made of plain
/// names only, so it cannot reach outside the working tree. A [`RelativePath`] is
/// `/`-separated on every platform; the Server turns it into a native path under the root.
pub fn valid_diff_path(path: &RelativePath) -> bool {
    !path.as_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, relative_path::Component::Normal(_)))
}

impl Pane {
    fn terminal(id: PaneId, cwd: PathBuf) -> Self {
        Self {
            id,
            content: PaneContent::Terminal {
                cwd: Some(cwd),
                agent_resume: None,
            },
        }
    }

    pub fn id(&self) -> PaneId {
        self.id
    }

    pub fn content(&self) -> &PaneContent {
        &self.content
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self.content, PaneContent::Terminal { .. })
    }

    /// `None` for a Pane that is no terminal.
    pub fn cwd(&self) -> Option<&Path> {
        match &self.content {
            PaneContent::Terminal { cwd, .. } => cwd.as_deref(),
            _ => None,
        }
    }

    pub fn agent_resume(&self) -> Option<&crate::AgentResume> {
        match &self.content {
            PaneContent::Terminal { agent_resume, .. } => agent_resume.as_ref(),
            _ => None,
        }
    }

    pub fn viewer(&self) -> Option<&Viewer> {
        match &self.content {
            PaneContent::Viewer(viewer) => Some(viewer),
            _ => None,
        }
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

    pub fn base_branch(&self) -> Option<&str> {
        self.base_branch.as_deref()
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
