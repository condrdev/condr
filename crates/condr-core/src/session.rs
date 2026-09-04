use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};

use crate::snapshot::{
    LayoutNodeSnapshot, LayoutSnapshot, MAX_SNAPSHOT_LAYOUT_DEPTH, MAX_SNAPSHOT_LAYOUT_NODES,
    MAX_SNAPSHOT_PANES, MAX_SNAPSHOT_TABS, MAX_SNAPSHOT_WORKSPACES, PaneSnapshot, SNAPSHOT_VERSION,
    SessionSnapshot, SnapshotError, TabSnapshot, WorkspaceSnapshot,
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
const MAX_STABLE_ID: u64 = u64::MAX / 2;
const MAX_NEXT_STABLE_ID_EXCLUSIVE: u64 = MAX_STABLE_ID + 1;
const MAX_NEXT_TAB_NUMBER_EXCLUSIVE: u64 = u64::MAX - 1;

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
    next_tab_number: u64,
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

    pub fn activate_workspace(&mut self, workspace_id: WorkspaceId) -> bool {
        if self
            .workspaces
            .iter()
            .any(|workspace| workspace.id == workspace_id)
        {
            self.active_workspace = Some(workspace_id);
            true
        } else {
            false
        }
    }

    pub fn activate_tab(&mut self, tab_id: TabId) -> bool {
        for workspace in &mut self.workspaces {
            if workspace.tabs.iter().any(|tab| tab.id == tab_id) {
                workspace.active_tab = tab_id;
                self.active_workspace = Some(workspace.id);
                return true;
            }
        }
        false
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

    pub fn create_workspace(&mut self, root_directory: PathBuf) -> Option<WorkspaceId> {
        if self.workspaces.len() >= MAX_SNAPSHOT_WORKSPACES
            || self.tab_count() >= MAX_SNAPSHOT_TABS
            || self.pane_count() >= MAX_SNAPSHOT_PANES
        {
            return None;
        }
        let first_id = reserve_ids(3)?;
        let workspace_id = WorkspaceId(first_id);
        let tab_id = TabId(first_id + 1);
        let pane_id = PaneId(first_id + 2);
        let workspace = Workspace {
            id: workspace_id,
            name: workspace_name(&root_directory),
            root_directory: root_directory.clone(),
            worktree: None,
            tabs: vec![Tab {
                id: tab_id,
                name: "Tab 1".into(),
                panes: vec![Pane {
                    id: pane_id,
                    cwd: Some(root_directory),
                }],
                focused_pane: pane_id,
                focus_history: Vec::new(),
                layout: PaneLayout::Pane(pane_id),
                zoomed_pane: None,
            }],
            active_tab: tab_id,
            next_tab_number: 2,
        };

        self.workspaces.push(workspace);
        self.active_workspace = Some(workspace_id);
        Some(workspace_id)
    }

    pub fn create_tab(&mut self, workspace_id: WorkspaceId) -> Option<TabId> {
        if self.tab_count() >= MAX_SNAPSHOT_TABS || self.pane_count() >= MAX_SNAPSHOT_PANES {
            return None;
        }
        let workspace_ix = self
            .workspaces
            .iter()
            .position(|workspace| workspace.id == workspace_id)?;
        let workspace = &mut self.workspaces[workspace_ix];
        let cwd = workspace
            .active_tab()
            .focused_pane()
            .cwd
            .clone()
            .unwrap_or_else(|| workspace.root_directory.clone());
        let tab_number = workspace.next_tab_number;
        let next_tab_number = tab_number
            .checked_add(1)
            .filter(|next| *next < MAX_NEXT_TAB_NUMBER_EXCLUSIVE)?;
        let first_id = reserve_ids(2)?;
        let tab_id = TabId(first_id);
        let pane_id = PaneId(first_id + 1);
        workspace.next_tab_number = next_tab_number;
        workspace.tabs.push(Tab {
            id: tab_id,
            name: format!("Tab {tab_number}"),
            panes: vec![Pane {
                id: pane_id,
                cwd: Some(cwd),
            }],
            focused_pane: pane_id,
            focus_history: Vec::new(),
            layout: PaneLayout::Pane(pane_id),
            zoomed_pane: None,
        });
        workspace.active_tab = tab_id;
        self.active_workspace = Some(workspace_id);
        Some(tab_id)
    }

    pub fn split_pane(
        &mut self,
        pane_id: PaneId,
        direction: SplitDirection,
        ratio: f32,
    ) -> Option<PaneId> {
        if self.pane_count() >= MAX_SNAPSHOT_PANES {
            return None;
        }
        let (workspace_ix, tab_ix, pane_ix) = self.find_pane(pane_id)?;
        let workspace = &mut self.workspaces[workspace_ix];
        let workspace_id = workspace.id;
        let tab_id = workspace.tabs[tab_ix].id;
        let cwd = workspace.tabs[tab_ix].panes[pane_ix]
            .cwd
            .clone()
            .unwrap_or_else(|| workspace.root_directory.clone());
        if pane_layout_depth(&workspace.tabs[tab_ix].layout, pane_id, 1)?
            >= MAX_SNAPSHOT_LAYOUT_DEPTH
        {
            return None;
        }
        let new_pane_id = PaneId(reserve_ids(1)?);
        let tab = &mut workspace.tabs[tab_ix];
        if !split_layout(
            &mut tab.layout,
            pane_id,
            new_pane_id,
            direction,
            valid_split_ratio(ratio),
        ) {
            return None;
        }

        let previous_focus = tab.focused_pane;
        tab.focus_history.retain(|id| *id != previous_focus);
        tab.focus_history.push(previous_focus);
        tab.focused_pane = new_pane_id;
        tab.zoomed_pane = None;
        tab.panes.push(Pane {
            id: new_pane_id,
            cwd: Some(cwd),
        });
        workspace.active_tab = tab_id;
        self.active_workspace = Some(workspace_id);
        Some(new_pane_id)
    }

    pub fn associate_worktree(
        &mut self,
        workspace_id: WorkspaceId,
        parent_workspace_id: WorkspaceId,
        parent_root_directory: PathBuf,
        managed: bool,
    ) -> bool {
        if workspace_id == parent_workspace_id || parent_root_directory.as_os_str().is_empty() {
            return false;
        }
        let Some(workspace) = self
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == workspace_id)
        else {
            return false;
        };
        workspace.worktree = Some(WorktreeAssociation {
            parent_workspace_id,
            parent_root_directory,
            managed,
        });
        true
    }

    pub fn clear_worktree_association(&mut self, workspace_id: WorkspaceId) -> bool {
        let Some(workspace) = self
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == workspace_id)
        else {
            return false;
        };
        workspace.worktree.take().is_some()
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

    pub fn set_pane_cwd(&mut self, pane_id: PaneId, cwd: Option<PathBuf>) -> bool {
        for workspace in &mut self.workspaces {
            for tab in &mut workspace.tabs {
                if let Some(pane) = tab.panes.iter_mut().find(|pane| pane.id == pane_id) {
                    pane.cwd = cwd;
                    return true;
                }
            }
        }
        false
    }

    pub fn focus_pane(&mut self, pane_id: PaneId) -> bool {
        let Some((workspace_ix, tab_ix, _)) = self.find_pane(pane_id) else {
            return false;
        };
        let workspace_id = self.workspaces[workspace_ix].id;
        let tab_id = self.workspaces[workspace_ix].tabs[tab_ix].id;
        self.active_workspace = Some(workspace_id);
        self.workspaces[workspace_ix].active_tab = tab_id;
        let tab = &mut self.workspaces[workspace_ix].tabs[tab_ix];
        let previous_focus = tab.focused_pane;
        if previous_focus != pane_id {
            tab.focus_history
                .retain(|id| *id != pane_id && *id != previous_focus);
            tab.focus_history.push(previous_focus);
            tab.focused_pane = pane_id;
        }
        true
    }

    pub fn close_pane(&mut self, pane_id: PaneId) -> Option<CloseOutcome> {
        let (workspace_ix, tab_ix, pane_ix) = self.find_pane(pane_id)?;
        let workspace_id = self.workspaces[workspace_ix].id;
        let tab_id = self.workspaces[workspace_ix].tabs[tab_ix].id;
        let mut outcome = CloseOutcome {
            panes: vec![pane_id],
            ..CloseOutcome::default()
        };

        if self.workspaces[workspace_ix].tabs[tab_ix].panes.len() > 1 {
            let tab = &mut self.workspaces[workspace_ix].tabs[tab_ix];
            tab.layout = remove_from_layout(&tab.layout, pane_id)
                .expect("a multi-pane layout remains after one pane closes");
            tab.panes.remove(pane_ix);
            tab.focus_history.retain(|id| *id != pane_id);
            if tab.zoomed_pane == Some(pane_id) {
                tab.zoomed_pane = None;
            }
            if tab.focused_pane == pane_id {
                tab.focused_pane = tab
                    .focus_history
                    .pop()
                    .filter(|id| tab.panes.iter().any(|pane| pane.id == *id))
                    .unwrap_or_else(|| first_pane_id(&tab.layout));
            }
            return Some(outcome);
        }

        let workspace = &mut self.workspaces[workspace_ix];
        workspace.tabs.remove(tab_ix);
        outcome.tabs.push(tab_id);
        if !workspace.tabs.is_empty() {
            if workspace.active_tab == tab_id {
                workspace.active_tab = workspace
                    .tabs
                    .get(tab_ix)
                    .or_else(|| workspace.tabs.last())
                    .expect("workspace has a remaining tab")
                    .id;
            }
            return Some(outcome);
        }

        self.workspaces.remove(workspace_ix);
        outcome.workspaces.push(workspace_id);
        if self.active_workspace == Some(workspace_id) {
            self.active_workspace = self
                .workspaces
                .get(workspace_ix)
                .or_else(|| self.workspaces.last())
                .map(|workspace| workspace.id);
        }
        Some(outcome)
    }

    pub fn close_tab(&mut self, tab_id: TabId) -> Option<CloseOutcome> {
        for workspace_ix in 0..self.workspaces.len() {
            let Some(tab_ix) = self.workspaces[workspace_ix]
                .tabs
                .iter()
                .position(|tab| tab.id == tab_id)
            else {
                continue;
            };
            if self.workspaces[workspace_ix].tabs.len() == 1 {
                return self.close_workspace(self.workspaces[workspace_ix].id);
            }

            let tab = self.workspaces[workspace_ix].tabs.remove(tab_ix);
            let outcome = CloseOutcome {
                panes: tab.panes.into_iter().map(|pane| pane.id).collect(),
                tabs: vec![tab_id],
                workspaces: Vec::new(),
            };
            let workspace = &mut self.workspaces[workspace_ix];
            if workspace.active_tab == tab_id {
                workspace.active_tab = workspace
                    .tabs
                    .get(tab_ix)
                    .or_else(|| workspace.tabs.last())
                    .expect("workspace has a remaining tab")
                    .id;
            }
            return Some(outcome);
        }
        None
    }

    pub fn close_workspace(&mut self, workspace_id: WorkspaceId) -> Option<CloseOutcome> {
        let workspace_ix = self
            .workspaces
            .iter()
            .position(|workspace| workspace.id == workspace_id)?;
        let workspace = self.workspaces.remove(workspace_ix);
        let mut panes = Vec::new();
        let mut tabs = Vec::new();
        for tab in workspace.tabs {
            tabs.push(tab.id);
            panes.extend(tab.panes.into_iter().map(|pane| pane.id));
        }
        if self.active_workspace == Some(workspace_id) {
            self.active_workspace = self
                .workspaces
                .get(workspace_ix)
                .or_else(|| self.workspaces.last())
                .map(|workspace| workspace.id);
        }
        Some(CloseOutcome {
            panes,
            tabs,
            workspaces: vec![workspace_id],
        })
    }

    pub fn move_workspace(&mut self, workspace_id: WorkspaceId, target_ix: usize) -> bool {
        let Some(source_ix) = self
            .workspaces
            .iter()
            .position(|workspace| workspace.id == workspace_id)
        else {
            return false;
        };
        if target_ix >= self.workspaces.len() || source_ix == target_ix {
            return false;
        }
        let workspace = self.workspaces.remove(source_ix);
        self.workspaces.insert(target_ix, workspace);
        true
    }

    pub fn move_tab(&mut self, tab_id: TabId, target_ix: usize) -> bool {
        for workspace in &mut self.workspaces {
            let Some(source_ix) = workspace.tabs.iter().position(|tab| tab.id == tab_id) else {
                continue;
            };
            if target_ix >= workspace.tabs.len() || source_ix == target_ix {
                return false;
            }
            let tab = workspace.tabs.remove(source_ix);
            workspace.tabs.insert(target_ix, tab);
            return true;
        }
        false
    }

    pub fn rename_workspace(&mut self, workspace_id: WorkspaceId, name: impl Into<String>) -> bool {
        let name = name.into();
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        let Some(workspace) = self
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == workspace_id)
        else {
            return false;
        };
        workspace.name = name.to_owned();
        true
    }

    pub fn rename_tab(&mut self, tab_id: TabId, name: impl Into<String>) -> bool {
        let name = name.into();
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        for workspace in &mut self.workspaces {
            if let Some(tab) = workspace.tabs.iter_mut().find(|tab| tab.id == tab_id) {
                tab.name = name.to_owned();
                return true;
            }
        }
        false
    }

    pub fn focus_pane_in_direction(&mut self, pane_id: PaneId, direction: PaneDirection) -> bool {
        let Some((workspace_ix, tab_ix, _)) = self.find_pane(pane_id) else {
            return false;
        };
        let Some(neighbor) = neighbor_pane_id(
            &self.workspaces[workspace_ix].tabs[tab_ix].layout,
            pane_id,
            direction,
        ) else {
            return false;
        };
        self.focus_pane(neighbor)
    }

    pub fn resize_pane(&mut self, pane_id: PaneId, direction: PaneDirection, amount: f32) -> bool {
        if !amount.is_finite() || amount == 0.0 {
            return false;
        }
        let Some((workspace_ix, tab_ix, _)) = self.find_pane(pane_id) else {
            return false;
        };
        let tab = &mut self.workspaces[workspace_ix].tabs[tab_ix];
        // tmux semantics: the command moves a boundary of the pane in `direction`.
        // The trailing boundary (right or bottom) moves when the pane has one, else
        // the leading boundary, so a pane can shrink as well as grow and the same
        // key moves the same divider the same way from either side of it.
        let (trailing, leading) = match direction {
            PaneDirection::Left | PaneDirection::Right => {
                (PaneDirection::Right, PaneDirection::Left)
            }
            PaneDirection::Up | PaneDirection::Down => (PaneDirection::Down, PaneDirection::Up),
        };
        let (neighbor, pane_grows) =
            if let Some(neighbor) = neighbor_pane_id(&tab.layout, pane_id, trailing) {
                (neighbor, direction == trailing)
            } else if let Some(neighbor) = neighbor_pane_id(&tab.layout, pane_id, leading) {
                (neighbor, direction == leading)
            } else {
                return false;
            };
        let amount = amount.abs().min(0.4);
        if pane_grows {
            resize_between(&mut tab.layout, pane_id, neighbor, amount)
        } else {
            resize_between(&mut tab.layout, neighbor, pane_id, amount)
        }
    }

    pub fn swap_pane(&mut self, pane_id: PaneId, direction: PaneDirection) -> bool {
        let Some((workspace_ix, tab_ix, _)) = self.find_pane(pane_id) else {
            return false;
        };
        let tab = &mut self.workspaces[workspace_ix].tabs[tab_ix];
        let Some(neighbor) = neighbor_pane_id(&tab.layout, pane_id, direction) else {
            return false;
        };
        swap_layout_panes(&mut tab.layout, pane_id, neighbor);
        true
    }

    pub fn set_tab_split_ratios(&mut self, tab_id: TabId, ratios: &[f32]) -> bool {
        if ratios.iter().any(|ratio| !ratio.is_finite()) {
            return false;
        }
        for workspace in &mut self.workspaces {
            let Some(tab) = workspace.tabs.iter_mut().find(|tab| tab.id == tab_id) else {
                continue;
            };
            if split_count(&tab.layout) != ratios.len() {
                return false;
            }
            let previous = tab.layout.clone();
            let mut ratios = ratios.iter().copied();
            apply_split_ratios(&mut tab.layout, &mut ratios);
            return tab.layout != previous;
        }
        false
    }

    pub fn toggle_pane_zoom(&mut self, pane_id: PaneId) -> bool {
        if !self.focus_pane(pane_id) {
            return false;
        }
        let (workspace_ix, tab_ix, _) = self
            .find_pane(pane_id)
            .expect("focused Pane remains in the Session");
        let tab = &mut self.workspaces[workspace_ix].tabs[tab_ix];
        if tab.panes.len() == 1 {
            return false;
        }
        tab.zoomed_pane = (tab.zoomed_pane != Some(pane_id)).then_some(pane_id);
        true
    }

    pub fn snapshot(&self) -> SessionSnapshot {
        SessionSnapshot {
            version: SNAPSHOT_VERSION,
            workspaces: self
                .workspaces
                .iter()
                .map(|workspace| WorkspaceSnapshot {
                    id: workspace.id,
                    name: workspace.name.clone(),
                    root_directory: workspace.root_directory.clone(),
                    worktree: workspace.worktree.clone(),
                    tabs: workspace
                        .tabs
                        .iter()
                        .map(|tab| TabSnapshot {
                            id: tab.id,
                            name: tab.name.clone(),
                            panes: tab
                                .panes
                                .iter()
                                .map(|pane| PaneSnapshot {
                                    id: pane.id,
                                    cwd: pane.cwd.clone(),
                                })
                                .collect(),
                            focused_pane: tab.focused_pane,
                            focus_history: tab.focus_history.clone(),
                            layout: LayoutSnapshot::from_layout(&tab.layout),
                        })
                        .collect(),
                    active_tab: workspace.active_tab,
                    next_tab_number: workspace.next_tab_number,
                })
                .collect(),
            active_workspace: self.active_workspace,
        }
    }

    pub fn restore(snapshot: SessionSnapshot) -> Result<Self, SnapshotError> {
        if snapshot.version != SNAPSHOT_VERSION {
            return Err(SnapshotError::UnsupportedVersion(snapshot.version));
        }
        validate_snapshot_resources(&snapshot)?;

        let mut workspaces = Vec::with_capacity(snapshot.workspaces.len());
        for workspace in snapshot.workspaces {
            let mut tabs = Vec::with_capacity(workspace.tabs.len());
            for tab in workspace.tabs {
                tabs.push(Tab {
                    id: tab.id,
                    name: tab.name,
                    panes: tab
                        .panes
                        .into_iter()
                        .map(|pane| Pane {
                            id: pane.id,
                            cwd: pane.cwd,
                        })
                        .collect(),
                    focused_pane: tab.focused_pane,
                    focus_history: tab.focus_history,
                    layout: restore_layout(&tab.layout),
                    zoomed_pane: None,
                });
            }
            workspaces.push(Workspace {
                id: workspace.id,
                name: workspace.name,
                root_directory: workspace.root_directory,
                worktree: workspace.worktree,
                tabs,
                active_tab: workspace.active_tab,
                next_tab_number: workspace.next_tab_number,
            });
        }
        let session = Self {
            workspaces,
            active_workspace: snapshot.active_workspace,
        };
        let max_id = session.validate()?;
        let next_id = max_id
            .checked_add(1)
            .ok_or(SnapshotError::Invalid("stable ID space is exhausted"))?;
        NEXT_ID.fetch_max(next_id, Ordering::Relaxed);
        Ok(session)
    }

    fn validate(&self) -> Result<u64, SnapshotError> {
        if self.workspaces.is_empty() != self.active_workspace.is_none() {
            return Err(SnapshotError::Invalid("invalid active Workspace"));
        }
        if self.active_workspace.is_some_and(|active| {
            !self
                .workspaces
                .iter()
                .any(|workspace| workspace.id == active)
        }) {
            return Err(SnapshotError::Invalid("active Workspace is missing"));
        }

        let mut workspace_ids = HashSet::new();
        let mut tab_ids = HashSet::new();
        let mut pane_ids = HashSet::new();
        let mut max_id = 0;
        for workspace in &self.workspaces {
            validate_id(workspace.id.0, &mut max_id)?;
            if !workspace_ids.insert(workspace.id) {
                return Err(SnapshotError::Invalid("duplicate Workspace ID"));
            }
            if workspace.name.is_empty() || workspace.tabs.is_empty() {
                return Err(SnapshotError::Invalid("invalid Workspace"));
            }
            if let Some(worktree) = &workspace.worktree {
                validate_id(worktree.parent_workspace_id.0, &mut max_id)?;
                if worktree.parent_workspace_id == workspace.id
                    || worktree.parent_root_directory.as_os_str().is_empty()
                {
                    return Err(SnapshotError::Invalid("invalid Worktree association"));
                }
            }
            if !workspace
                .tabs
                .iter()
                .any(|tab| tab.id == workspace.active_tab)
            {
                return Err(SnapshotError::Invalid("active Tab is missing"));
            }
            if workspace.next_tab_number <= workspace.tabs.len() as u64
                || workspace.next_tab_number >= MAX_NEXT_TAB_NUMBER_EXCLUSIVE
            {
                return Err(SnapshotError::Invalid("invalid next Tab number"));
            }

            for tab in &workspace.tabs {
                validate_id(tab.id.0, &mut max_id)?;
                if !tab_ids.insert(tab.id) {
                    return Err(SnapshotError::Invalid("duplicate Tab ID"));
                }
                if tab.name.is_empty() || tab.panes.is_empty() {
                    return Err(SnapshotError::Invalid("invalid Tab"));
                }

                let mut tab_pane_ids = HashSet::new();
                for pane in &tab.panes {
                    validate_id(pane.id.0, &mut max_id)?;
                    if !pane_ids.insert(pane.id) || !tab_pane_ids.insert(pane.id) {
                        return Err(SnapshotError::Invalid("duplicate Pane ID"));
                    }
                }
                if !tab_pane_ids.contains(&tab.focused_pane) {
                    return Err(SnapshotError::Invalid("focused Pane is missing"));
                }
                let mut history = HashSet::new();
                if tab.focus_history.iter().any(|id| {
                    *id == tab.focused_pane || !tab_pane_ids.contains(id) || !history.insert(*id)
                }) {
                    return Err(SnapshotError::Invalid("invalid Pane focus history"));
                }
                let mut layout_panes = HashSet::new();
                validate_layout(&tab.layout, &mut layout_panes)?;
                if layout_panes != tab_pane_ids {
                    return Err(SnapshotError::Invalid("layout Pane set does not match Tab"));
                }
            }
        }
        Ok(max_id)
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

fn workspace_name(root_directory: &Path) -> String {
    root_directory
        .file_name()
        .filter(|name| !name.is_empty())
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| {
            let name = root_directory.display().to_string();
            if name.is_empty() {
                "Workspace".into()
            } else {
                name
            }
        })
}

fn split_layout(
    layout: &mut PaneLayout,
    target: PaneId,
    new_pane: PaneId,
    direction: SplitDirection,
    ratio: f32,
) -> bool {
    match layout {
        PaneLayout::Pane(id) if *id == target => {
            *layout = PaneLayout::Split {
                direction,
                ratio,
                first: Box::new(PaneLayout::Pane(target)),
                second: Box::new(PaneLayout::Pane(new_pane)),
            };
            true
        }
        PaneLayout::Pane(_) => false,
        PaneLayout::Split { first, second, .. } => {
            split_layout(first, target, new_pane, direction, ratio)
                || split_layout(second, target, new_pane, direction, ratio)
        }
    }
}

fn pane_layout_depth(layout: &PaneLayout, target: PaneId, depth: usize) -> Option<usize> {
    match layout {
        PaneLayout::Pane(id) => (*id == target).then_some(depth),
        PaneLayout::Split { first, second, .. } => pane_layout_depth(first, target, depth + 1)
            .or_else(|| pane_layout_depth(second, target, depth + 1)),
    }
}

fn valid_split_ratio(ratio: f32) -> f32 {
    if ratio.is_finite() {
        ratio.clamp(0.1, 0.9)
    } else {
        0.5
    }
}

fn remove_from_layout(layout: &PaneLayout, target: PaneId) -> Option<PaneLayout> {
    match layout {
        PaneLayout::Pane(id) if *id == target => None,
        PaneLayout::Pane(_) => Some(layout.clone()),
        PaneLayout::Split {
            direction,
            ratio,
            first,
            second,
        } => match (
            remove_from_layout(first, target),
            remove_from_layout(second, target),
        ) {
            (None, Some(second)) => Some(second),
            (Some(first), None) => Some(first),
            (Some(first), Some(second)) => Some(PaneLayout::Split {
                direction: *direction,
                ratio: *ratio,
                first: Box::new(first),
                second: Box::new(second),
            }),
            (None, None) => None,
        },
    }
}

fn first_pane_id(layout: &PaneLayout) -> PaneId {
    match layout {
        PaneLayout::Pane(id) => *id,
        PaneLayout::Split { first, .. } => first_pane_id(first),
    }
}

#[derive(Clone, Copy)]
struct PaneRect {
    id: PaneId,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

fn neighbor_pane_id(
    layout: &PaneLayout,
    target: PaneId,
    direction: PaneDirection,
) -> Option<PaneId> {
    let mut rects = Vec::new();
    collect_pane_rects(layout, 0.0, 0.0, 1.0, 1.0, &mut rects);
    let target = *rects.iter().find(|rect| rect.id == target)?;
    rects
        .into_iter()
        .filter(|candidate| candidate.id != target.id)
        .filter_map(|candidate| {
            directional_score(target, candidate, direction).map(|score| (candidate.id, score))
        })
        .min_by(|(_, left), (_, right)| left.total_cmp(right))
        .map(|(id, _)| id)
}

fn collect_pane_rects(
    layout: &PaneLayout,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    rects: &mut Vec<PaneRect>,
) {
    match layout {
        PaneLayout::Pane(id) => rects.push(PaneRect {
            id: *id,
            left,
            top,
            right,
            bottom,
        }),
        PaneLayout::Split {
            direction: SplitDirection::Horizontal,
            ratio,
            first,
            second,
        } => {
            let split = left + (right - left) * ratio;
            collect_pane_rects(first, left, top, split, bottom, rects);
            collect_pane_rects(second, split, top, right, bottom, rects);
        }
        PaneLayout::Split {
            direction: SplitDirection::Vertical,
            ratio,
            first,
            second,
        } => {
            let split = top + (bottom - top) * ratio;
            collect_pane_rects(first, left, top, right, split, rects);
            collect_pane_rects(second, left, split, right, bottom, rects);
        }
    }
}

fn directional_score(
    target: PaneRect,
    candidate: PaneRect,
    direction: PaneDirection,
) -> Option<f32> {
    let (primary, secondary, center) = match direction {
        PaneDirection::Left if candidate.right <= target.left + f32::EPSILON => (
            target.left - candidate.right,
            interval_gap(target.top, target.bottom, candidate.top, candidate.bottom),
            ((target.top + target.bottom) - (candidate.top + candidate.bottom)).abs(),
        ),
        PaneDirection::Right if candidate.left >= target.right - f32::EPSILON => (
            candidate.left - target.right,
            interval_gap(target.top, target.bottom, candidate.top, candidate.bottom),
            ((target.top + target.bottom) - (candidate.top + candidate.bottom)).abs(),
        ),
        PaneDirection::Up if candidate.bottom <= target.top + f32::EPSILON => (
            target.top - candidate.bottom,
            interval_gap(target.left, target.right, candidate.left, candidate.right),
            ((target.left + target.right) - (candidate.left + candidate.right)).abs(),
        ),
        PaneDirection::Down if candidate.top >= target.bottom - f32::EPSILON => (
            candidate.top - target.bottom,
            interval_gap(target.left, target.right, candidate.left, candidate.right),
            ((target.left + target.right) - (candidate.left + candidate.right)).abs(),
        ),
        _ => return None,
    };
    Some(primary * 100.0 + secondary * 10.0 + center)
}

fn interval_gap(first_start: f32, first_end: f32, second_start: f32, second_end: f32) -> f32 {
    if first_end < second_start {
        second_start - first_end
    } else if second_end < first_start {
        first_start - second_end
    } else {
        0.0
    }
}

fn contains_pane(layout: &PaneLayout, pane_id: PaneId) -> bool {
    match layout {
        PaneLayout::Pane(id) => *id == pane_id,
        PaneLayout::Split { first, second, .. } => {
            contains_pane(first, pane_id) || contains_pane(second, pane_id)
        }
    }
}

fn resize_between(layout: &mut PaneLayout, target: PaneId, neighbor: PaneId, amount: f32) -> bool {
    let PaneLayout::Split {
        ratio,
        first,
        second,
        ..
    } = layout
    else {
        return false;
    };
    let target_first = contains_pane(first, target);
    let neighbor_first = contains_pane(first, neighbor);
    if target_first != neighbor_first {
        let next = valid_split_ratio(*ratio + if target_first { amount } else { -amount });
        if next == *ratio {
            return false;
        }
        *ratio = next;
        return true;
    }
    if target_first {
        resize_between(first, target, neighbor, amount)
    } else {
        resize_between(second, target, neighbor, amount)
    }
}

fn swap_layout_panes(layout: &mut PaneLayout, first_id: PaneId, second_id: PaneId) {
    match layout {
        PaneLayout::Pane(id) if *id == first_id => *id = second_id,
        PaneLayout::Pane(id) if *id == second_id => *id = first_id,
        PaneLayout::Pane(_) => {}
        PaneLayout::Split { first, second, .. } => {
            swap_layout_panes(first, first_id, second_id);
            swap_layout_panes(second, first_id, second_id);
        }
    }
}

fn split_count(layout: &PaneLayout) -> usize {
    match layout {
        PaneLayout::Pane(_) => 0,
        PaneLayout::Split { first, second, .. } => 1 + split_count(first) + split_count(second),
    }
}

fn apply_split_ratios(layout: &mut PaneLayout, ratios: &mut impl Iterator<Item = f32>) {
    if let PaneLayout::Split {
        ratio,
        first,
        second,
        ..
    } = layout
    {
        *ratio = valid_split_ratio(ratios.next().expect("split ratio count was checked"));
        apply_split_ratios(first, ratios);
        apply_split_ratios(second, ratios);
    }
}

fn validate_snapshot_resources(snapshot: &SessionSnapshot) -> Result<(), SnapshotError> {
    if snapshot.workspaces.len() > MAX_SNAPSHOT_WORKSPACES {
        return Err(SnapshotError::Invalid("too many Workspaces"));
    }

    let mut tab_count = 0usize;
    let mut pane_count = 0usize;
    let mut layout_node_count = 0usize;
    for workspace in &snapshot.workspaces {
        tab_count = tab_count
            .checked_add(workspace.tabs.len())
            .ok_or(SnapshotError::Invalid("too many Tabs"))?;
        if tab_count > MAX_SNAPSHOT_TABS {
            return Err(SnapshotError::Invalid("too many Tabs"));
        }
        for tab in &workspace.tabs {
            pane_count = pane_count
                .checked_add(tab.panes.len())
                .ok_or(SnapshotError::Invalid("too many Panes"))?;
            if pane_count > MAX_SNAPSHOT_PANES {
                return Err(SnapshotError::Invalid("too many Panes"));
            }
            layout_node_count = layout_node_count
                .checked_add(tab.layout.nodes.len())
                .ok_or(SnapshotError::Invalid("too many layout nodes"))?;
            if layout_node_count > MAX_SNAPSHOT_LAYOUT_NODES {
                return Err(SnapshotError::Invalid("too many layout nodes"));
            }
        }
    }

    for workspace in &snapshot.workspaces {
        for tab in &workspace.tabs {
            let pane_ids = tab.panes.iter().map(|pane| pane.id).collect::<HashSet<_>>();
            if pane_ids.len() != tab.panes.len() {
                return Err(SnapshotError::Invalid("duplicate Pane ID"));
            }
            validate_layout_snapshot(&tab.layout, &pane_ids)?;
        }
    }
    Ok(())
}

fn validate_layout_snapshot(
    layout: &LayoutSnapshot,
    expected_panes: &HashSet<PaneId>,
) -> Result<(), SnapshotError> {
    let expected_node_count = expected_panes
        .len()
        .checked_mul(2)
        .and_then(|count| count.checked_sub(1))
        .ok_or(SnapshotError::Invalid("invalid empty layout"))?;
    if layout.nodes.len() != expected_node_count {
        return Err(SnapshotError::Invalid(
            "layout node count does not match Tab",
        ));
    }

    let root = usize::try_from(layout.root)
        .map_err(|_| SnapshotError::Invalid("layout root is out of bounds"))?;
    if root >= layout.nodes.len() {
        return Err(SnapshotError::Invalid("layout root is out of bounds"));
    }

    let mut states = vec![0u8; layout.nodes.len()];
    let mut stack = vec![(root, 1usize, false)];
    let mut layout_panes = HashSet::new();
    while let Some((index, depth, exiting)) = stack.pop() {
        if index >= layout.nodes.len() {
            return Err(SnapshotError::Invalid("layout node is out of bounds"));
        }
        if exiting {
            states[index] = 2;
            continue;
        }
        match states[index] {
            1 => return Err(SnapshotError::Invalid("cycle in layout")),
            2 => return Err(SnapshotError::Invalid("duplicate layout node reference")),
            _ => {}
        }
        if depth > MAX_SNAPSHOT_LAYOUT_DEPTH {
            return Err(SnapshotError::Invalid("layout exceeds maximum depth"));
        }

        states[index] = 1;
        match &layout.nodes[index] {
            LayoutNodeSnapshot::Pane(pane_id) => {
                if !layout_panes.insert(*pane_id) {
                    return Err(SnapshotError::Invalid("duplicate Pane in layout"));
                }
                states[index] = 2;
            }
            LayoutNodeSnapshot::Split {
                ratio,
                first,
                second,
                ..
            } => {
                if !ratio.is_finite() || !(0.1..=0.9).contains(ratio) {
                    return Err(SnapshotError::Invalid("invalid split ratio"));
                }
                let first = usize::try_from(*first)
                    .map_err(|_| SnapshotError::Invalid("layout node is out of bounds"))?;
                let second = usize::try_from(*second)
                    .map_err(|_| SnapshotError::Invalid("layout node is out of bounds"))?;
                stack.push((index, depth, true));
                stack.push((second, depth + 1, false));
                stack.push((first, depth + 1, false));
            }
        }
    }

    if states.contains(&0) {
        return Err(SnapshotError::Invalid("unreachable layout node"));
    }
    if &layout_panes != expected_panes {
        return Err(SnapshotError::Invalid("layout Pane set does not match Tab"));
    }
    Ok(())
}

fn restore_layout(layout: &LayoutSnapshot) -> PaneLayout {
    fn restore_node(nodes: &[LayoutNodeSnapshot], index: usize) -> PaneLayout {
        match &nodes[index] {
            LayoutNodeSnapshot::Pane(pane_id) => PaneLayout::Pane(*pane_id),
            LayoutNodeSnapshot::Split {
                direction,
                ratio,
                first,
                second,
            } => PaneLayout::Split {
                direction: *direction,
                ratio: *ratio,
                first: Box::new(restore_node(nodes, *first as usize)),
                second: Box::new(restore_node(nodes, *second as usize)),
            },
        }
    }

    restore_node(&layout.nodes, layout.root as usize)
}

fn validate_id(id: u64, max_id: &mut u64) -> Result<(), SnapshotError> {
    if id == 0 || id > MAX_STABLE_ID {
        return Err(SnapshotError::Invalid("invalid stable ID"));
    }
    *max_id = (*max_id).max(id);
    Ok(())
}

fn validate_layout(
    layout: &PaneLayout,
    pane_ids: &mut HashSet<PaneId>,
) -> Result<(), SnapshotError> {
    match layout {
        PaneLayout::Pane(id) => {
            if !pane_ids.insert(*id) {
                return Err(SnapshotError::Invalid("duplicate Pane in layout"));
            }
        }
        PaneLayout::Split {
            ratio,
            first,
            second,
            ..
        } => {
            if !ratio.is_finite() || !(0.1..=0.9).contains(ratio) {
                return Err(SnapshotError::Invalid("invalid split ratio"));
            }
            validate_layout(first, pane_ids)?;
            validate_layout(second, pane_ids)?;
        }
    }
    Ok(())
}
