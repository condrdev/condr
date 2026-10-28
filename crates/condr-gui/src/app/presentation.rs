use super::*;

impl Condr {
    /// The Workspace connection `key` shows (ADR 0021).
    pub(super) fn presented_workspace_id(
        &self,
        key: ConnectionKey,
        session: &Session,
    ) -> Option<WorkspaceId> {
        self.connection(key)?.viewed_workspace_id(session)
    }

    /// The connection, Session, Workspace and Tab the active window shows, when it shows
    /// one: what its actions apply to.
    pub(super) fn presented(&self) -> Option<(ConnectionKey, &Session, WorkspaceId, TabId)> {
        self.presented_in(self.active_view())
    }

    /// Shows `workspace_id` (and `tab_id`) on connection `key`, in whichever window draws
    /// it; when that is the main window, it presents that connection too. Returns whether
    /// anything shown changed; the caller rebuilds the Dock when it did.
    pub(super) fn show_view(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: Option<TabId>,
    ) -> bool {
        // A Tab with a window of its own is shown there and moves nothing here, so a Diff
        // Tab on the other screen follows the Changes clicked in this one (ADR 0043).
        if tab_id.is_some_and(|tab_id| self.window_for(WindowScope::Tab(key, tab_id)).is_some()) {
            return false;
        }
        // A Workspace with a window of its own shows the Tab there, and the Device's own
        // choice of Workspace stays.
        let keep_workspace = self
            .window_for(WindowScope::Workspace(key, workspace_id))
            .is_some();
        let Some(connection) = self.connection_mut(key) else {
            return false;
        };
        let previous_workspace = connection.view_workspace;
        let view_changed = connection.set_view(workspace_id, tab_id);
        if keep_workspace {
            connection.view_workspace = previous_workspace;
        }
        let shown_tab = tab_id.or_else(|| {
            let connection = self.connection(key)?;
            connection.viewed_tab_id(connection.session()?, workspace_id)
        });
        let in_main = self.home_window(key, Some(workspace_id), shown_tab) == self.main_window();
        let connection_changed = in_main && self.active_connection != key;
        if in_main {
            self.active_connection = key;
        }
        self.refresh_target_pane(key);
        if view_changed || connection_changed {
            self.state_dirty = true;
        }
        view_changed || connection_changed
    }

    pub(super) fn has_pending_projection_for(&self, key: ConnectionKey) -> bool {
        self.dock_surfaces.iter().any(|(surface_key, surface)| {
            surface_key.connection_key == key && surface.pending_projection_request.is_some()
        })
    }

    pub(super) fn clear_pending_projections_for(&mut self, key: ConnectionKey) -> bool {
        let mut active_projection_cleared = false;
        for (surface_key, surface) in &mut self.dock_surfaces {
            if surface_key.connection_key == key && surface.pending_projection_request.is_some() {
                surface.pending_projection_request = None;
                surface.pending_projection_applied_sequence = None;
                surface.projection = None;
                active_projection_cleared |= self
                    .windows
                    .iter()
                    .any(|view| view.dock_surface == Some(*surface_key));
            }
        }
        active_projection_cleared
    }

    /// Structure at `sequence` is now authoritative: projections whose Layout command
    /// landed at or before it are settled. Returns whether a drawn surface's was.
    pub(super) fn resolve_projections_at(&mut self, key: ConnectionKey, sequence: u64) -> bool {
        let mut active_projection_resolved = false;
        for (surface_key, surface) in &mut self.dock_surfaces {
            if surface_key.connection_key == key
                && surface
                    .pending_projection_applied_sequence
                    .is_some_and(|applied| applied <= sequence)
            {
                surface.pending_projection_request = None;
                surface.pending_projection_applied_sequence = None;
                active_projection_resolved |= self
                    .windows
                    .iter()
                    .any(|view| view.dock_surface == Some(*surface_key));
            }
        }
        active_projection_resolved
    }

    /// The structure at `sequence` is applied: settle whatever waited for it, point the
    /// target Pane at the new structure, and say whether the Dock needs rebuilding. Both
    /// the `LayoutChanged` event and a late `LayoutApplied` end here.
    pub(super) fn settle_layout(
        &mut self,
        key: ConnectionKey,
        sequence: u64,
        layout_changed: bool,
    ) -> IncomingEffect {
        let active_projection_resolved = self.resolve_projections_at(key, sequence);
        let target_before_refresh = self.terminal_input.target;
        self.refresh_target_pane(key);
        let target_changed = self.terminal_input.target != target_before_refresh;
        IncomingEffect {
            rebuild: layout_changed || active_projection_resolved || target_changed,
            rebuild_active: false,
            notify: true,
        }
    }

    pub(super) fn clear_connection_gui_state(&mut self, key: ConnectionKey, cx: &mut App) {
        self.clear_files_state(key, cx);
        self.retain_dock_cache(key, &HashSet::new(), &HashSet::new());
    }

    /// Presentation caches share one retention rule for structure changes and detach.
    pub(super) fn retain_dock_cache(
        &mut self,
        key: ConnectionKey,
        tab_ids: &HashSet<TabId>,
        pane_ids: &HashSet<PaneId>,
    ) {
        self.dock_surfaces.retain(|surface, _| {
            surface.connection_key != key || tab_ids.contains(&surface.tab_id)
        });
        self.panels.retain(|(connection_key, pane_id), _| {
            *connection_key != key || pane_ids.contains(pane_id)
        });
        self.pending_sizes.retain(|(connection_key, pane_id), _| {
            *connection_key != key || pane_ids.contains(pane_id)
        });
        self.terminal_geometry
            .retain(|(connection_key, pane_id), _| {
                *connection_key != key || pane_ids.contains(pane_id)
            });
        for view in &mut self.windows {
            if view.dock_surface.is_some_and(|surface| {
                surface.connection_key == key && !tab_ids.contains(&surface.tab_id)
            }) {
                view.dock_surface = None;
            }
        }
        self.terminal_input.retain_panes(|connection_key, pane_id| {
            connection_key != key || pane_ids.contains(&pane_id)
        });
    }
}
