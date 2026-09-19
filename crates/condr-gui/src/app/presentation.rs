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

    /// The Tab connection `key` shows in `workspace_id`.
    pub(super) fn presented_tab_id(
        &self,
        key: ConnectionKey,
        session: &Session,
        workspace_id: WorkspaceId,
    ) -> Option<TabId> {
        self.connection(key)?.viewed_tab_id(session, workspace_id)
    }

    /// The connection, Session, Workspace and Tab the window shows, when it shows one.
    pub(super) fn presented(&self) -> Option<(ConnectionKey, Session, WorkspaceId, TabId)> {
        let connection = self.active_connection()?;
        let session = connection.session()?;
        let (workspace_id, tab_id) = connection.viewed(&session)?;
        Some((connection.key, session, workspace_id, tab_id))
    }

    /// Shows `workspace_id` (and `tab_id`) on connection `key` and makes that connection
    /// the active one. Returns whether anything shown changed; the caller rebuilds the
    /// Dock when it did.
    pub(super) fn show_view(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: Option<TabId>,
    ) -> bool {
        let Some(connection) = self.connection_mut(key) else {
            return false;
        };
        let view_changed = connection.set_view(workspace_id, tab_id);
        let connection_changed = self.active_connection != key;
        self.active_connection = key;
        self.refresh_target_pane(key);
        view_changed || connection_changed
    }

    pub(super) fn has_pending_projection_for(&self, key: ConnectionKey) -> bool {
        self.dock_surfaces.iter().any(|(surface_key, surface)| {
            surface_key.connection_key == key && surface.pending_projection_request.is_some()
        })
    }

    pub(super) fn clear_pending_projections_for(&mut self, key: ConnectionKey) -> bool {
        let active_surface = self.active_dock_surface;
        let mut active_projection_cleared = false;
        for (surface_key, surface) in &mut self.dock_surfaces {
            if surface_key.connection_key == key && surface.pending_projection_request.is_some() {
                surface.pending_projection_request = None;
                surface.pending_projection_applied_sequence = None;
                surface.projection = None;
                active_projection_cleared |= active_surface == Some(*surface_key);
            }
        }
        active_projection_cleared
    }

    /// Structure at `sequence` is now authoritative: projections whose Layout command
    /// landed at or before it are settled. Returns whether the active surface's was.
    pub(super) fn resolve_projections_at(&mut self, key: ConnectionKey, sequence: u64) -> bool {
        let active_surface = self.active_dock_surface;
        let mut active_projection_resolved = false;
        for (surface_key, surface) in &mut self.dock_surfaces {
            if surface_key.connection_key == key
                && surface
                    .pending_projection_applied_sequence
                    .is_some_and(|applied| applied <= sequence)
            {
                surface.pending_projection_request = None;
                surface.pending_projection_applied_sequence = None;
                active_projection_resolved |= active_surface == Some(*surface_key);
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
        let target_before_refresh = self.target_pane;
        self.refresh_target_pane(key);
        let target_changed =
            self.active_connection == key && self.target_pane != target_before_refresh;
        IncomingEffect {
            rebuild: layout_changed || active_projection_resolved || target_changed,
            rebuild_active: false,
            notify: true,
        }
    }

    pub(super) fn clear_connection_gui_state(&mut self, key: ConnectionKey) {
        self.clear_files_state(key);
        self.dock_surfaces
            .retain(|surface, _| surface.connection_key != key);
        if self
            .active_dock_surface
            .is_some_and(|surface| surface.connection_key == key)
        {
            self.active_dock_surface = None;
        }
        self.panels
            .retain(|(connection_key, _), _| *connection_key != key);
        self.pending_sizes
            .retain(|(connection_key, _), _| *connection_key != key);
        self.terminal_geometry
            .retain(|(connection_key, _), _| *connection_key != key);
        if self
            .terminal_selection
            .is_some_and(|selection| selection.connection_key == key)
        {
            self.terminal_selection = None;
        }
        if self
            .hovered_link
            .as_ref()
            .is_some_and(|(connection_key, _, _)| *connection_key == key)
        {
            self.hovered_link = None;
        }
        if self
            .pressed_terminal_link
            .as_ref()
            .is_some_and(|(connection_key, _, _)| *connection_key == key)
        {
            self.pressed_terminal_link = None;
        }
        if self
            .terminal_mouse_capture
            .is_some_and(|capture| capture.connection_key == key)
        {
            self.terminal_mouse_capture = None;
        }
        if self
            .last_terminal_mouse_motion
            .is_some_and(|motion| motion.connection_key == key)
        {
            self.last_terminal_mouse_motion = None;
        }
        if self
            .focused_terminal
            .is_some_and(|(connection_key, _)| connection_key == key)
        {
            self.focused_terminal = None;
        }
        if self
            .reported_terminal_focus
            .is_some_and(|(connection_key, _)| connection_key == key)
        {
            self.reported_terminal_focus = None;
        }
        if self
            .target_pane
            .is_some_and(|(connection_key, _)| connection_key == key)
        {
            self.target_pane = None;
        }
        if self
            .terminal_composition
            .as_ref()
            .is_some_and(|composition| composition.connection_key == key)
        {
            self.terminal_composition = None;
        }
    }
}
