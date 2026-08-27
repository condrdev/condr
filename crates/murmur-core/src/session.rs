use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use serde::{Deserialize, Serialize};

use crate::snapshot::{
    PaneSnapshot, SNAPSHOT_VERSION, SessionSnapshot, SnapshotError, TabSnapshot, WorkspaceSnapshot,
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn next_id() -> u64 {
    NEXT_ID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkspaceId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TabId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PaneId(u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SplitDirection {
    Horizontal,
    Vertical,
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

#[derive(Debug, Default)]
pub struct Session {
    workspaces: Vec<Workspace>,
    active_workspace: Option<WorkspaceId>,
}

#[derive(Debug)]
pub struct Workspace {
    id: WorkspaceId,
    name: String,
    root_directory: PathBuf,
    tabs: Vec<Tab>,
    active_tab: TabId,
    next_tab_number: u64,
}

#[derive(Debug)]
pub struct Tab {
    id: TabId,
    name: String,
    panes: Vec<Pane>,
    focused_pane: PaneId,
    focus_history: Vec<PaneId>,
    layout: PaneLayout,
}

#[derive(Debug)]
pub struct Pane {
    id: PaneId,
    cwd: Option<PathBuf>,
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

    pub fn create_workspace(&mut self, root_directory: PathBuf) -> WorkspaceId {
        let workspace_id = WorkspaceId(next_id());
        let tab_id = TabId(next_id());
        let pane_id = PaneId(next_id());
        let workspace = Workspace {
            id: workspace_id,
            name: workspace_name(&root_directory),
            root_directory: root_directory.clone(),
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
            }],
            active_tab: tab_id,
            next_tab_number: 2,
        };

        self.workspaces.push(workspace);
        self.active_workspace = Some(workspace_id);
        workspace_id
    }

    pub fn create_tab(&mut self, workspace_id: WorkspaceId) -> Option<TabId> {
        let workspace = self
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == workspace_id)?;
        let cwd = workspace
            .active_tab()
            .focused_pane()
            .cwd
            .clone()
            .unwrap_or_else(|| workspace.root_directory.clone());
        let tab_id = TabId(next_id());
        let pane_id = PaneId(next_id());
        let tab_number = workspace.next_tab_number;
        workspace.next_tab_number += 1;
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
        let (workspace_ix, tab_ix, pane_ix) = self.find_pane(pane_id)?;
        let workspace = &mut self.workspaces[workspace_ix];
        let workspace_id = workspace.id;
        let tab_id = workspace.tabs[tab_ix].id;
        let cwd = workspace.tabs[tab_ix].panes[pane_ix]
            .cwd
            .clone()
            .unwrap_or_else(|| workspace.root_directory.clone());
        let new_pane_id = PaneId(next_id());
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
        tab.panes.push(Pane {
            id: new_pane_id,
            cwd: Some(cwd),
        });
        workspace.active_tab = tab_id;
        self.active_workspace = Some(workspace_id);
        Some(new_pane_id)
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
                            layout: tab.layout.clone(),
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
        let session = Self {
            workspaces: snapshot
                .workspaces
                .into_iter()
                .map(|workspace| Workspace {
                    id: workspace.id,
                    name: workspace.name,
                    root_directory: workspace.root_directory,
                    tabs: workspace
                        .tabs
                        .into_iter()
                        .map(|tab| Tab {
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
                            layout: tab.layout,
                        })
                        .collect(),
                    active_tab: workspace.active_tab,
                    next_tab_number: workspace.next_tab_number,
                })
                .collect(),
            active_workspace: snapshot.active_workspace,
        };
        let max_id = session.validate()?;
        NEXT_ID.fetch_max(max_id + 1, Ordering::Relaxed);
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
            if !workspace
                .tabs
                .iter()
                .any(|tab| tab.id == workspace.active_tab)
            {
                return Err(SnapshotError::Invalid("active Tab is missing"));
            }
            if workspace.next_tab_number <= workspace.tabs.len() as u64 {
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
}

impl Pane {
    pub fn id(&self) -> PaneId {
        self.id
    }

    pub fn cwd(&self) -> Option<&Path> {
        self.cwd.as_deref()
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

fn validate_id(id: u64, max_id: &mut u64) -> Result<(), SnapshotError> {
    if id == 0 || id == u64::MAX {
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
