use super::*;

impl Session {
    /// Replaces the native conversation to resume; reports whether it changed.
    pub fn set_pane_agent_resume(
        &mut self,
        pane_id: PaneId,
        resume: Option<crate::AgentResume>,
    ) -> bool {
        if resume
            .as_ref()
            .is_some_and(|resume| !crate::agent::valid_session_id(&resume.session_id))
        {
            return false;
        }
        let Some((workspace, tab, pane)) = self.find_pane(pane_id) else {
            return false;
        };
        let pane = &mut self.workspaces[workspace].tabs[tab].panes[pane];
        if pane.agent_resume == resume {
            return false;
        }
        pane.agent_resume = resume;
        true
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
        let side = match direction {
            SplitDirection::Horizontal => PaneDirection::Right,
            SplitDirection::Vertical => PaneDirection::Down,
        };
        if !split_layout(
            &mut tab.layout,
            pane_id,
            new_pane_id,
            side,
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
            agent_resume: None,
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

    /// Detaches `pane_id` from its Tab layout and reattaches it beside `target` on `side`,
    /// like tmux `join-pane`. Both Panes must share a Tab; a Tab's only Pane cannot move.
    pub fn move_pane(&mut self, pane_id: PaneId, target: PaneId, side: PaneDirection) -> bool {
        if pane_id == target {
            return false;
        }
        let Some((workspace_ix, tab_ix, _)) = self.find_pane(pane_id) else {
            return false;
        };
        let tab = &mut self.workspaces[workspace_ix].tabs[tab_ix];
        if !tab.panes.iter().any(|pane| pane.id == target) {
            return false;
        }
        let Some(mut layout) = remove_from_layout(&tab.layout, pane_id) else {
            return false;
        };
        if pane_layout_depth(&layout, target, 1).is_none_or(|d| d >= MAX_SNAPSHOT_LAYOUT_DEPTH) {
            return false;
        }
        if !split_layout(&mut layout, target, pane_id, side, 0.5) {
            return false;
        }
        tab.layout = layout;
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
}
