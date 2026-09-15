use super::*;
use crate::snapshot::TabContentSnapshot;

impl Session {
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
                            content: match &tab.content {
                                TabContent::Terminals(terminals) => TabContentSnapshot::Terminals {
                                    panes: terminals
                                        .panes
                                        .iter()
                                        .map(|pane| PaneSnapshot {
                                            id: pane.id,
                                            cwd: pane.cwd.clone(),
                                            agent_resume: pane.agent_resume.clone(),
                                        })
                                        .collect(),
                                    focused_pane: terminals.focused_pane,
                                    focus_history: terminals.focus_history.clone(),
                                    layout: LayoutSnapshot::from_layout(&terminals.layout),
                                },
                                TabContent::Diff(diff) => TabContentSnapshot::Diff {
                                    path: diff.path.clone(),
                                },
                            },
                        })
                        .collect(),
                    active_tab: workspace.active_tab,
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
                let content = match tab.content {
                    TabContentSnapshot::Terminals {
                        panes,
                        focused_pane,
                        focus_history,
                        layout,
                    } => TabContent::Terminals(TerminalLayout {
                        panes: panes
                            .into_iter()
                            .map(|pane| Pane {
                                id: pane.id,
                                cwd: pane.cwd,
                                agent_resume: pane.agent_resume,
                            })
                            .collect(),
                        focused_pane,
                        focus_history,
                        layout: restore_layout(&layout),
                        zoomed_pane: None,
                    }),
                    TabContentSnapshot::Diff { path } => TabContent::Diff(DiffView { path }),
                };
                tabs.push(Tab {
                    id: tab.id,
                    name: tab.name,
                    content,
                });
            }
            workspaces.push(Workspace {
                id: workspace.id,
                name: workspace.name,
                root_directory: workspace.root_directory,
                worktree: workspace.worktree,
                tabs,
                active_tab: workspace.active_tab,
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
            let mut diff_tabs = 0;
            for tab in &workspace.tabs {
                validate_id(tab.id.0, &mut max_id)?;
                if !tab_ids.insert(tab.id) {
                    return Err(SnapshotError::Invalid("duplicate Tab ID"));
                }
                let terminals = match &tab.content {
                    TabContent::Terminals(terminals) => terminals,
                    TabContent::Diff(diff) => {
                        if !valid_diff_path(&diff.path) {
                            return Err(SnapshotError::Invalid("invalid Diff Tab path"));
                        }
                        diff_tabs += 1;
                        if diff_tabs > 1 {
                            return Err(SnapshotError::Invalid(
                                "a Workspace has at most one Diff Tab",
                            ));
                        }
                        continue;
                    }
                };
                if terminals.panes.is_empty() {
                    return Err(SnapshotError::Invalid("invalid Tab"));
                }

                let mut tab_pane_ids = HashSet::new();
                for pane in &terminals.panes {
                    validate_id(pane.id.0, &mut max_id)?;
                    if pane
                        .agent_resume
                        .as_ref()
                        .is_some_and(|resume| !crate::agent::valid_session_id(&resume.session_id))
                    {
                        return Err(SnapshotError::Invalid("invalid Agent resume ID"));
                    }
                    if !pane_ids.insert(pane.id) || !tab_pane_ids.insert(pane.id) {
                        return Err(SnapshotError::Invalid("duplicate Pane ID"));
                    }
                }
                if !tab_pane_ids.contains(&terminals.focused_pane) {
                    return Err(SnapshotError::Invalid("focused Pane is missing"));
                }
                let mut history = HashSet::new();
                if terminals.focus_history.iter().any(|id| {
                    *id == terminals.focused_pane
                        || !tab_pane_ids.contains(id)
                        || !history.insert(*id)
                }) {
                    return Err(SnapshotError::Invalid("invalid Pane focus history"));
                }
                let mut layout_panes = HashSet::new();
                validate_layout(&terminals.layout, &mut layout_panes)?;
                if layout_panes != tab_pane_ids {
                    return Err(SnapshotError::Invalid("layout Pane set does not match Tab"));
                }
            }
        }
        Ok(max_id)
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
            let TabContentSnapshot::Terminals { panes, layout, .. } = &tab.content else {
                continue;
            };
            pane_count = pane_count
                .checked_add(panes.len())
                .ok_or(SnapshotError::Invalid("too many Panes"))?;
            if pane_count > MAX_SNAPSHOT_PANES {
                return Err(SnapshotError::Invalid("too many Panes"));
            }
            layout_node_count = layout_node_count
                .checked_add(layout.nodes.len())
                .ok_or(SnapshotError::Invalid("too many layout nodes"))?;
            if layout_node_count > MAX_SNAPSHOT_LAYOUT_NODES {
                return Err(SnapshotError::Invalid("too many layout nodes"));
            }
        }
    }

    for workspace in &snapshot.workspaces {
        for tab in &workspace.tabs {
            let TabContentSnapshot::Terminals { panes, layout, .. } = &tab.content else {
                continue;
            };
            let pane_ids = panes.iter().map(|pane| pane.id).collect::<HashSet<_>>();
            if pane_ids.len() != panes.len() {
                return Err(SnapshotError::Invalid("duplicate Pane ID"));
            }
            validate_layout_snapshot(layout, &pane_ids)?;
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
