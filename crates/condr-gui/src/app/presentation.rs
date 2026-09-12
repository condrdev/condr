use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PendingWorkspaceSelection {
    pub(super) connection_key: ConnectionKey,
    pub(super) workspace_id: WorkspaceId,
    pub(super) pane_id: Option<PaneId>,
    pub(super) connect_generation: u64,
    pub(super) server_id: ServerId,
    pub(super) runtime_epoch: RuntimeEpoch,
    pub(super) session_id: SessionId,
    pub(super) request_id: u64,
    pub(super) applied_sequence: Option<u64>,
}

impl PendingWorkspaceSelection {
    pub(super) fn belongs_to(self, connection: &ServerConnection) -> bool {
        self.connection_key == connection.key
            && self.connect_generation == connection.connect_generation
            && Some(self.server_id) == connection.server_id
            && Some(self.runtime_epoch) == connection.runtime_epoch
            && Some(self.session_id) == connection.session_id
    }
}

impl Condr {
    pub(super) fn pending_workspace_selection_for(
        &self,
        key: ConnectionKey,
    ) -> Option<PendingWorkspaceSelection> {
        let pending = *self.pending_workspace_selections.get(&key)?;
        let connection = self.connection(key)?;
        pending.belongs_to(connection).then_some(pending)
    }

    pub(super) fn has_pending_presentation(&self) -> bool {
        self.pending_presentation_request
            .and_then(|(key, request_id)| {
                self.pending_workspace_selection_for(key)
                    .filter(|pending| pending.request_id == request_id)
            })
            .is_some()
    }

    pub(super) fn should_hold_active_surface(&self) -> bool {
        self.pending_workspace_selection_for(self.active_connection)
            .is_some()
            || self.has_pending_presentation()
    }

    pub(super) fn workspace_id_for_surface(
        session: &Session,
        surface: DockSurfaceKey,
    ) -> Option<WorkspaceId> {
        session
            .workspaces()
            .iter()
            .find(|workspace| {
                workspace
                    .tabs()
                    .iter()
                    .any(|tab| tab.id() == surface.tab_id)
            })
            .map(|workspace| workspace.id())
    }

    pub(super) fn presented_workspace_id(
        &self,
        key: ConnectionKey,
        session: &Session,
    ) -> Option<WorkspaceId> {
        if self.active_connection == key
            && let Some(surface) = self
                .active_dock_surface
                .filter(|surface| surface.connection_key == key)
            && let Some(workspace_id) = Self::workspace_id_for_surface(session, surface)
        {
            return Some(workspace_id);
        }
        session.active_workspace_id()
    }

    pub(super) fn presented_tab_id(
        &self,
        key: ConnectionKey,
        session: &Session,
        workspace_id: WorkspaceId,
    ) -> Option<TabId> {
        let workspace = session.workspace(workspace_id)?;
        if self.active_connection == key
            && let Some(surface) = self
                .active_dock_surface
                .filter(|surface| surface.connection_key == key)
            && workspace
                .tabs()
                .iter()
                .any(|tab| tab.id() == surface.tab_id)
        {
            return Some(surface.tab_id);
        }
        Some(workspace.active_tab().id())
    }

    pub(super) fn clear_pending_workspace_selection_for(&mut self, key: ConnectionKey) -> bool {
        let removed = self.pending_workspace_selections.remove(&key);
        if removed.is_some_and(|pending| {
            self.pending_presentation_request == Some((key, pending.request_id))
        }) {
            self.pending_presentation_request = None;
        }
        removed.is_some()
    }

    pub(super) fn cancel_pending_presentation(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let was_holding = self.should_hold_active_surface();
        let cancelled = self.pending_presentation_request.take().is_some();
        if cancelled && was_holding && !self.should_hold_active_surface() {
            self.refresh_target_pane(self.active_connection);
            self.rebuild_dock(window, cx);
        }
        if cancelled {
            cx.notify();
        }
        cancelled
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

    /// Settles a pending Workspace selection whose command landed at or before
    /// `sequence`: `(ready, failed)` for the active connection, both false otherwise.
    pub(super) fn resolve_pending_workspace_at(
        &mut self,
        key: ConnectionKey,
        index: usize,
        sequence: u64,
    ) -> (bool, bool) {
        let session = Session::restore(self.connections[index].snapshot.clone()).ok();
        let (Some(pending), Some(session)) =
            (self.pending_workspace_selection_for(key), session.as_ref())
        else {
            return (false, false);
        };
        // The layout event/bootstrap is itself the authoritative result.  The direct
        // LayoutApplied acknowledgement can be lost when the reliable writer falls behind;
        // waiting for it here would leave the selection pending forever and disable every
        // subsequent layout action until the GUI reconnects.  Only defer when an acknowledgement
        // explicitly points at a newer sequence than the snapshot being considered.
        if pending
            .applied_sequence
            .is_some_and(|applied| applied > sequence)
        {
            return (false, false);
        }
        let target_is_active = session.active_workspace_id() == Some(pending.workspace_id)
            && pending.pane_id.is_none_or(|pane_id| {
                session
                    .active_workspace()
                    .is_some_and(|workspace| workspace.active_tab().focused_pane().id() == pane_id)
            });
        // Without an acknowledgement, keep waiting while the authoritative structure still
        // points elsewhere.  This preserves an in-flight, supersedable click while allowing a
        // matching event/bootstrap to settle it when the direct reply was lost.
        if pending.applied_sequence.is_none() && !target_is_active {
            return (false, false);
        }
        self.pending_workspace_selections.remove(&key);
        let should_present = self.pending_presentation_request == Some((key, pending.request_id));
        if should_present {
            self.pending_presentation_request = None;
        }
        if target_is_active {
            if should_present {
                self.active_connection = key;
            }
            (self.active_connection == key, false)
        } else {
            (false, self.active_connection == key)
        }
    }

    /// The structure at `sequence` is applied: settle whatever waited for it, point the
    /// target Pane at the new structure, and say whether the Dock needs rebuilding. Both
    /// the `LayoutChanged` event and a late `LayoutApplied` end here.
    pub(super) fn settle_layout(
        &mut self,
        key: ConnectionKey,
        index: usize,
        sequence: u64,
        layout_changed: bool,
    ) -> IncomingEffect {
        let active_projection_resolved = self.resolve_projections_at(key, sequence);
        let (pending_workspace_ready, pending_workspace_failed) =
            self.resolve_pending_workspace_at(key, index, sequence);
        let preserve_visible_workspace = self.active_connection == key
            && self.should_hold_active_surface()
            && self
                .active_dock_surface
                .is_some_and(|surface| surface.connection_key == key);
        let target_before_refresh = self.target_pane;
        if !preserve_visible_workspace {
            self.refresh_target_pane(key);
        }
        let target_changed =
            self.active_connection == key && self.target_pane != target_before_refresh;
        IncomingEffect {
            rebuild: !preserve_visible_workspace
                && (layout_changed
                    || pending_workspace_ready
                    || pending_workspace_failed
                    || active_projection_resolved
                    || target_changed),
            rebuild_active: false,
            notify: true,
        }
    }

    pub(super) fn clear_connection_gui_state(&mut self, key: ConnectionKey) {
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
