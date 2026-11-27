use super::*;

impl Session {
    /// Keeps every existing Tab's Pane focus, history and zoom when a split does not
    /// request focus; the new Pane still exists, it just does not take the focus.
    pub fn preserve_focus_from(&mut self, previous: &Session) {
        for workspace in &mut self.workspaces {
            if let Some(old) = previous.workspace(workspace.id) {
                for tab in &mut workspace.tabs {
                    let Some(old) = old.tabs.iter().find(|old| old.id == tab.id) else {
                        continue;
                    };
                    let tab = &mut tab.layout;
                    let present = |id: &PaneId| tab.panes.iter().any(|pane| pane.id == *id);
                    // Only what is still in this Tab: a Pane may have moved to another.
                    if present(&old.layout.focused_pane) {
                        let history: Vec<PaneId> = old
                            .layout
                            .focus_history
                            .iter()
                            .copied()
                            .filter(present)
                            .collect();
                        tab.focused_pane = old.layout.focused_pane;
                        tab.focus_history = history;
                        tab.zoomed_pane = old.layout.zoomed_pane.filter(present);
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
                layout: TabLayout::single(Pane::terminal(pane_id, root_directory)),
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
            .and_then(|pane| pane.cwd().map(Path::to_path_buf))
            .unwrap_or_else(|| workspace.root_directory.clone());
        let first_id = reserve_ids(2)?;
        let tab_id = TabId(first_id);
        let pane_id = PaneId(first_id + 1);
        workspace.tabs.push(Tab {
            id: tab_id,
            name: String::new(),
            layout: TabLayout::single(Pane::terminal(pane_id, cwd)),
        });
        Some(tab_id)
    }

    /// Shows `viewer` in the Workspace's Pane of its kind: retargeted where it is, which
    /// takes its Tab's focus, or the first time in a Tab of its own (ADR 0078). `None` for an
    /// unknown Workspace, an invalid path, or when the Tab or Pane limit is reached.
    pub fn show_viewer(
        &mut self,
        workspace_id: WorkspaceId,
        viewer: Viewer,
    ) -> Option<(TabId, PaneId)> {
        if !viewer.valid() {
            return None;
        }
        let workspace_ix = self
            .workspaces
            .iter()
            .position(|workspace| workspace.id == workspace_id)?;
        let shown = self.workspaces[workspace_ix].tabs.iter().find_map(|tab| {
            tab.panes()
                .iter()
                .find(|pane| {
                    pane.viewer()
                        .is_some_and(|shown| shown.kind() == viewer.kind())
                })
                .map(|pane| (tab.id, pane.id))
        });
        if let Some((tab_id, pane_id)) = shown {
            let (workspace_ix, tab_ix, pane_ix) = self.find_pane(pane_id)?;
            self.tab_layout_mut(workspace_ix, tab_ix).panes[pane_ix].content =
                PaneContent::Viewer(viewer);
            self.focus_pane(pane_id);
            return Some((tab_id, pane_id));
        }
        if self.tab_count() >= MAX_SNAPSHOT_TABS || self.pane_count() >= MAX_SNAPSHOT_PANES {
            return None;
        }
        let first_id = reserve_ids(2)?;
        let (tab_id, pane_id) = (TabId(first_id), PaneId(first_id + 1));
        self.workspaces[workspace_ix].tabs.push(Tab {
            id: tab_id,
            name: String::new(),
            layout: TabLayout::single(Pane {
                id: pane_id,
                content: PaneContent::Viewer(viewer),
            }),
        });
        Some((tab_id, pane_id))
    }

    /// Drops the Panes of kinds this build does not know, with a Tab or Workspace they leave
    /// empty, as a Server does with a Snapshot a newer one wrote (ADR 0078); how many went.
    pub fn drop_unsupported_panes(&mut self) -> usize {
        let unsupported: Vec<PaneId> = self
            .workspaces
            .iter()
            .flat_map(Workspace::tabs)
            .flat_map(Tab::panes)
            .filter(|pane| pane.content == PaneContent::Unsupported)
            .map(Pane::id)
            .collect();
        for pane_id in &unsupported {
            self.close_pane(*pane_id);
        }
        unsupported.len()
    }

    /// Points the Workspace's Preview Pane at `to` when it shows `from` or a path under it,
    /// after a rename (ADR 0045). `false` when nothing changed.
    pub fn follow_rename(
        &mut self,
        workspace_id: WorkspaceId,
        from: &RelativePath,
        to: &RelativePath,
    ) -> bool {
        let Some(workspace) = self
            .workspaces
            .iter_mut()
            .find(|workspace| workspace.id == workspace_id)
        else {
            return false;
        };
        let mut changed = false;
        for pane in workspace
            .tabs
            .iter_mut()
            .flat_map(|tab| tab.layout.panes.iter_mut())
        {
            if let PaneContent::Viewer(Viewer::File { path }) = &mut pane.content
                && let Ok(rest) = path.strip_prefix(from)
            {
                *path = if rest.as_str().is_empty() {
                    to.to_owned()
                } else {
                    to.join(rest)
                };
                changed = true;
            }
        }
        changed
    }

    pub fn associate_worktree(
        &mut self,
        workspace_id: WorkspaceId,
        parent_workspace_id: WorkspaceId,
        parent_root_directory: PathBuf,
        managed: bool,
        base_branch: Option<String>,
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
            base_branch,
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
