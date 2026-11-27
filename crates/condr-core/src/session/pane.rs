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
        let PaneContent::Terminal { agent_resume, .. } =
            &mut self.tab_layout_mut(workspace, tab).panes[pane].content
        else {
            return false;
        };
        if *agent_resume == resume {
            return false;
        }
        *agent_resume = resume;
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
        let root_directory = workspace.root_directory.clone();
        let tab = &mut workspace.tabs[tab_ix].layout;
        // A viewer has no directory: the new terminal starts in the root (ADR 0078).
        let cwd = tab.panes[pane_ix]
            .cwd()
            .map_or(root_directory, Path::to_path_buf);
        if pane_layout_depth(&tab.layout, pane_id, 1)? >= MAX_SNAPSHOT_LAYOUT_DEPTH {
            return None;
        }
        let new_pane_id = PaneId(reserve_ids(1)?);
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
        tab.panes.push(Pane::terminal(new_pane_id, cwd));
        Some(new_pane_id)
    }

    /// A terminal Pane's directory; `false` for any other Pane.
    pub fn set_pane_cwd(&mut self, pane_id: PaneId, cwd: Option<PathBuf>) -> bool {
        let Some((workspace, tab, pane)) = self.find_pane(pane_id) else {
            return false;
        };
        match &mut self.tab_layout_mut(workspace, tab).panes[pane].content {
            PaneContent::Terminal { cwd: current, .. } => {
                *current = cwd;
                true
            }
            _ => false,
        }
    }

    pub fn focus_pane(&mut self, pane_id: PaneId) -> bool {
        let Some((workspace_ix, tab_ix, _)) = self.find_pane(pane_id) else {
            return false;
        };
        let tab = self.tab_layout_mut(workspace_ix, tab_ix);
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

        if self.workspaces[workspace_ix].tabs[tab_ix].panes().len() > 1 {
            let tab = self.tab_layout_mut(workspace_ix, tab_ix);
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
            return Some(outcome);
        }

        self.workspaces.remove(workspace_ix);
        outcome.workspaces.push(workspace_id);
        Some(outcome)
    }

    pub fn focus_pane_in_direction(&mut self, pane_id: PaneId, direction: PaneDirection) -> bool {
        let Some((workspace_ix, tab_ix, _)) = self.find_pane(pane_id) else {
            return false;
        };
        let Some(neighbor) = neighbor_pane_id(
            &self.tab_layout_mut(workspace_ix, tab_ix).layout,
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
        let tab = self.tab_layout_mut(workspace_ix, tab_ix);
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
        let tab = self.tab_layout_mut(workspace_ix, tab_ix);
        let Some(neighbor) = neighbor_pane_id(&tab.layout, pane_id, direction) else {
            return false;
        };
        swap_layout_panes(&mut tab.layout, pane_id, neighbor);
        true
    }

    /// Exchanges two Panes' places in their Tab; every split keeps its direction and ratio.
    pub fn swap_panes(&mut self, pane_id: PaneId, other: PaneId) -> bool {
        if pane_id == other {
            return false;
        }
        let Some((workspace_ix, tab_ix, _)) = self.find_pane(pane_id) else {
            return false;
        };
        let tab = self.tab_layout_mut(workspace_ix, tab_ix);
        if !tab.panes.iter().any(|pane| pane.id == other) {
            return false;
        }
        swap_layout_panes(&mut tab.layout, pane_id, other);
        true
    }

    /// Detaches `pane_id` from its Tab's layout and reattaches it beside `target` on `side`,
    /// like tmux `join-pane`, in any Tab of the same Workspace; a Tab the Pane leaves empty
    /// closes (ADR 0078).
    pub fn move_pane(&mut self, pane_id: PaneId, target: PaneId, side: PaneDirection) -> bool {
        if pane_id == target {
            return false;
        }
        let (Some((workspace_ix, from_ix, _)), Some((target_workspace_ix, to_ix, _))) =
            (self.find_pane(pane_id), self.find_pane(target))
        else {
            return false;
        };
        if workspace_ix != target_workspace_ix {
            return false;
        }
        // Checked before anything moves, so a refusal leaves the Session as it was.
        let tabs = &self.workspaces[workspace_ix].tabs;
        let target_layout = if from_ix == to_ix {
            let Some(layout) = remove_from_layout(&tabs[from_ix].layout.layout, pane_id) else {
                return false;
            };
            layout
        } else {
            tabs[to_ix].layout.layout.clone()
        };
        if pane_layout_depth(&target_layout, target, 1)
            .is_none_or(|depth| depth >= MAX_SNAPSHOT_LAYOUT_DEPTH)
        {
            return false;
        }
        // Within its Tab only the layout changes; focus and zoom stay as they are.
        if from_ix == to_ix {
            let mut layout = target_layout;
            if !split_layout(&mut layout, target, pane_id, side, 0.5) {
                return false;
            }
            self.workspaces[workspace_ix].tabs[from_ix].layout.layout = layout;
            return true;
        }
        let Some(pane) = self.detach_pane(workspace_ix, from_ix, pane_id, false) else {
            return false;
        };
        let moved = TabLayout::single(pane);
        let (_, to_ix, _) = self.find_pane(target).expect("the target stays");
        self.attach_layout(workspace_ix, to_ix, target, moved, side)
    }

    /// Puts all of Tab `tab_id` beside `target`, a Pane of another Tab of its Workspace, as
    /// one side of a split, and closes it (ADR 0078).
    pub fn move_tab_into(&mut self, tab_id: TabId, target: PaneId, side: PaneDirection) -> bool {
        let Some((workspace_ix, to_ix, _)) = self.find_pane(target) else {
            return false;
        };
        let workspace = &self.workspaces[workspace_ix];
        let Some(from_ix) = workspace.tabs.iter().position(|tab| tab.id == tab_id) else {
            return false;
        };
        if from_ix == to_ix {
            return false;
        }
        let moved = &workspace.tabs[from_ix].layout;
        let depth = layout_depth(&moved.layout);
        let tab = &workspace.tabs[to_ix].layout;
        if pane_layout_depth(&tab.layout, target, 1)
            .is_none_or(|d| d + depth > MAX_SNAPSHOT_LAYOUT_DEPTH)
        {
            return false;
        }
        let moved = self.workspaces[workspace_ix].tabs.remove(from_ix).layout;
        let (_, to_ix, _) = self.find_pane(target).expect("the target stays");
        self.attach_layout(workspace_ix, to_ix, target, moved, side)
    }

    /// Makes `pane_id` a Tab of its own, right after the one it leaves; `None` for a Tab's
    /// only Pane (ADR 0078).
    pub fn move_pane_to_new_tab(&mut self, pane_id: PaneId) -> Option<TabId> {
        let (workspace_ix, tab_ix, _) = self.find_pane(pane_id)?;
        if self.workspaces[workspace_ix].tabs[tab_ix]
            .layout
            .panes
            .len()
            == 1
            || self.tab_count() >= MAX_SNAPSHOT_TABS
        {
            return None;
        }
        let tab_id = TabId(reserve_ids(1)?);
        let pane = self.detach_pane(workspace_ix, tab_ix, pane_id, true)?;
        self.workspaces[workspace_ix].tabs.insert(
            tab_ix + 1,
            Tab {
                id: tab_id,
                name: String::new(),
                layout: TabLayout::single(pane),
            },
        );
        Some(tab_id)
    }

    /// Takes `pane_id` out of Tab `tab_ix`'s layout, which closes when it empties, unless
    /// `keep_tab` says another Pane of it is the target.
    fn detach_pane(
        &mut self,
        workspace_ix: usize,
        tab_ix: usize,
        pane_id: PaneId,
        keep_tab: bool,
    ) -> Option<Pane> {
        let tab = &mut self.workspaces[workspace_ix].tabs[tab_ix].layout;
        let pane_ix = tab.panes.iter().position(|pane| pane.id == pane_id)?;
        match remove_from_layout(&tab.layout, pane_id) {
            Some(layout) => {
                tab.layout = layout;
                tab.focus_history.retain(|id| *id != pane_id);
                if tab.zoomed_pane == Some(pane_id) {
                    tab.zoomed_pane = None;
                }
                if tab.focused_pane == pane_id {
                    tab.focused_pane = tab
                        .focus_history
                        .pop()
                        .unwrap_or_else(|| first_pane_id(&tab.layout));
                }
                Some(tab.panes.remove(pane_ix))
            }
            None if keep_tab => None,
            None => Some(
                self.workspaces[workspace_ix]
                    .tabs
                    .remove(tab_ix)
                    .layout
                    .panes
                    .remove(pane_ix),
            ),
        }
    }

    /// Splits `target` in Tab `tab_ix` with `moved` on `side`, which takes the focus.
    fn attach_layout(
        &mut self,
        workspace_ix: usize,
        tab_ix: usize,
        target: PaneId,
        moved: TabLayout,
        side: PaneDirection,
    ) -> bool {
        let tab = &mut self.workspaces[workspace_ix].tabs[tab_ix].layout;
        if !split_layout_with(&mut tab.layout, target, moved.layout, side, 0.5) {
            return false;
        }
        let previous = tab.focused_pane;
        tab.focus_history.retain(|id| *id != previous);
        tab.focus_history.push(previous);
        tab.focused_pane = moved.focused_pane;
        tab.zoomed_pane = None;
        tab.panes.extend(moved.panes);
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
            let tab = &mut tab.layout;
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
        let tab = self.tab_layout_mut(workspace_ix, tab_ix);
        if tab.panes.len() == 1 {
            return false;
        }
        tab.zoomed_pane = (tab.zoomed_pane != Some(pane_id)).then_some(pane_id);
        true
    }
}
