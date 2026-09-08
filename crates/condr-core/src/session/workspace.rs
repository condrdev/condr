use super::*;

impl Session {
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

    /// Keeps existing selection and focus history when a staged creation does not request
    /// focus. Newly created Workspaces/Tabs retain their valid initial selection.
    pub fn preserve_selection_from(&mut self, previous: &Session) {
        if let Some(active) = previous.active_workspace {
            self.active_workspace = Some(active);
        }
        for workspace in &mut self.workspaces {
            if let Some(old) = previous.workspace(workspace.id) {
                workspace.active_tab = old.active_tab;
                for tab in &mut workspace.tabs {
                    if let Some(old) = old.tabs.iter().find(|old| old.id == tab.id) {
                        tab.focused_pane = old.focused_pane;
                        tab.focus_history.clone_from(&old.focus_history);
                        tab.zoomed_pane = old.zoomed_pane;
                    }
                }
            }
        }
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
                name: String::new(),
                panes: vec![Pane {
                    id: pane_id,
                    cwd: Some(root_directory),
                    agent_resume: None,
                }],
                focused_pane: pane_id,
                focus_history: Vec::new(),
                layout: PaneLayout::Pane(pane_id),
                zoomed_pane: None,
            }],
            active_tab: tab_id,
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
        let first_id = reserve_ids(2)?;
        let tab_id = TabId(first_id);
        let pane_id = PaneId(first_id + 1);
        workspace.tabs.push(Tab {
            id: tab_id,
            name: String::new(),
            panes: vec![Pane {
                id: pane_id,
                cwd: Some(cwd),
                agent_resume: None,
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
