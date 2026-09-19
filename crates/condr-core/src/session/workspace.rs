use super::*;

/// The display name every Diff Tab starts with; the Tab is identified by its content, not by
/// this name, which the user may change.
pub const DIFF_TAB_NAME: &str = "Diff";
/// The same for the Preview Tab (ADR 0018).
pub const FILE_TAB_NAME: &str = "Preview";

impl Session {
    /// Keeps every existing Tab's Pane focus, history and zoom when a split does not
    /// request focus; the new Pane still exists, it just does not take the focus.
    pub fn preserve_focus_from(&mut self, previous: &Session) {
        for workspace in &mut self.workspaces {
            if let Some(old) = previous.workspace(workspace.id) {
                for tab in &mut workspace.tabs {
                    let old = old.tabs.iter().find(|old| old.id == tab.id);
                    if let (Some(terminals), Some(old)) =
                        (tab.terminals_mut(), old.and_then(Tab::terminals))
                    {
                        terminals.focused_pane = old.focused_pane;
                        terminals.focus_history.clone_from(&old.focus_history);
                        terminals.zoomed_pane = old.zoomed_pane;
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
                content: TabContent::Terminals(TerminalLayout::single(Pane {
                    id: pane_id,
                    cwd: Some(root_directory),
                    agent_resume: None,
                })),
            }],
        };

        self.workspaces.push(workspace);
        Some(workspace_id)
    }

    /// The new Tab's shell starts where `cwd_from` is, when that Pane belongs to the
    /// Workspace (the caller's own Pane, or the one it is looking at); otherwise in the
    /// Workspace root. Which Pane that is cannot be read off the Session, since each
    /// client shows its own Tab (ADR 0021).
    pub fn create_tab(
        &mut self,
        workspace_id: WorkspaceId,
        cwd_from: Option<PaneId>,
    ) -> Option<TabId> {
        if self.tab_count() >= MAX_SNAPSHOT_TABS || self.pane_count() >= MAX_SNAPSHOT_PANES {
            return None;
        }
        let workspace_ix = self
            .workspaces
            .iter()
            .position(|workspace| workspace.id == workspace_id)?;
        let workspace = &mut self.workspaces[workspace_ix];
        let cwd = cwd_from
            .and_then(|pane_id| {
                workspace
                    .tabs
                    .iter()
                    .flat_map(Tab::panes)
                    .find(|pane| pane.id == pane_id)
            })
            .and_then(|pane| pane.cwd.clone())
            .unwrap_or_else(|| workspace.root_directory.clone());
        let first_id = reserve_ids(2)?;
        let tab_id = TabId(first_id);
        let pane_id = PaneId(first_id + 1);
        workspace.tabs.push(Tab {
            id: tab_id,
            name: String::new(),
            content: TabContent::Terminals(TerminalLayout::single(Pane {
                id: pane_id,
                cwd: Some(cwd),
                agent_resume: None,
            })),
        });
        Some(tab_id)
    }

    /// Shows `path`'s diff in the Workspace's Diff Tab, creating the Tab the first time and
    /// retargeting it afterwards (ADR 0017). `None` for an unknown Workspace, an invalid
    /// path, or when the Tab limit is reached.
    pub fn show_diff(&mut self, workspace_id: WorkspaceId, path: RelativePathBuf) -> Option<TabId> {
        self.show_viewer(
            workspace_id,
            TabContent::Diff(DiffView { path }),
            DIFF_TAB_NAME,
        )
    }

    /// Shows `path`'s content in the Workspace's Preview Tab, the same way (ADR 0018).
    pub fn show_file(&mut self, workspace_id: WorkspaceId, path: RelativePathBuf) -> Option<TabId> {
        self.show_viewer(
            workspace_id,
            TabContent::File(FileView { path }),
            FILE_TAB_NAME,
        )
    }

    fn show_viewer(
        &mut self,
        workspace_id: WorkspaceId,
        content: TabContent,
        name: &str,
    ) -> Option<TabId> {
        let path = match &content {
            TabContent::Diff(diff) => &diff.path,
            TabContent::File(file) => &file.path,
            TabContent::Terminals(_) => return None,
        };
        if !valid_diff_path(path) {
            return None;
        }
        let workspace_ix = self
            .workspaces
            .iter()
            .position(|workspace| workspace.id == workspace_id)?;
        let workspace = &mut self.workspaces[workspace_ix];
        let tab_id = match workspace
            .tabs
            .iter_mut()
            .find(|tab| tab.content.same_viewer_kind(&content))
        {
            Some(tab) => {
                tab.content = content;
                tab.id
            }
            None => {
                if self.tab_count() >= MAX_SNAPSHOT_TABS {
                    return None;
                }
                let tab_id = TabId(reserve_ids(1)?);
                let workspace = &mut self.workspaces[workspace_ix];
                workspace.tabs.push(Tab {
                    id: tab_id,
                    name: name.to_owned(),
                    content,
                });
                tab_id
            }
        };
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
            return Some(CloseOutcome {
                panes: tab.panes().iter().map(|pane| pane.id).collect(),
                tabs: vec![tab_id],
                workspaces: Vec::new(),
            });
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
            panes.extend(tab.panes().iter().map(|pane| pane.id));
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
