use super::*;

impl Condr {
    pub(super) fn refresh_target_pane(&mut self, key: ConnectionKey) {
        let target = self.connection_focused_pane(key);
        if key == self.active_connection {
            self.target_pane = target.map(|pane_id| (key, pane_id));
            if let Some(pane_id) = target {
                self.mark_pane_seen(key, pane_id);
            }
        }
    }

    pub(super) fn prune_dock_cache(&mut self, key: ConnectionKey) {
        let Some(session) = self
            .connection(key)
            .and_then(|connection| Session::restore(connection.snapshot.clone()).ok())
        else {
            return;
        };
        let tab_ids = session
            .workspaces()
            .iter()
            .flat_map(|workspace| workspace.tabs())
            .map(|tab| tab.id())
            .collect::<HashSet<_>>();
        let pane_ids = session
            .workspaces()
            .iter()
            .flat_map(|workspace| workspace.tabs())
            .flat_map(|tab| tab.panes())
            .map(|pane| pane.id())
            .collect::<HashSet<_>>();

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
        if self.active_dock_surface.is_some_and(|surface| {
            surface.connection_key == key && !tab_ids.contains(&surface.tab_id)
        }) {
            self.active_dock_surface = None;
        }
        if self.terminal_selection.is_some_and(|selection| {
            selection.connection_key == key && !pane_ids.contains(&selection.pane_id)
        }) {
            self.terminal_selection = None;
        }
        if self
            .hovered_link
            .as_ref()
            .is_some_and(|(connection_key, pane_id, _)| {
                *connection_key == key && !pane_ids.contains(pane_id)
            })
        {
            self.hovered_link = None;
        }
        if self
            .pressed_terminal_link
            .as_ref()
            .is_some_and(|(connection_key, pane_id, _)| {
                *connection_key == key && !pane_ids.contains(pane_id)
            })
        {
            self.pressed_terminal_link = None;
        }
        if self.terminal_mouse_capture.is_some_and(|capture| {
            capture.connection_key == key && !pane_ids.contains(&capture.pane_id)
        }) {
            self.terminal_mouse_capture = None;
        }
        if self.last_terminal_mouse_motion.is_some_and(|motion| {
            motion.connection_key == key && !pane_ids.contains(&motion.pane_id)
        }) {
            self.last_terminal_mouse_motion = None;
        }
        if self
            .focused_terminal
            .is_some_and(|(connection_key, pane_id)| {
                connection_key == key && !pane_ids.contains(&pane_id)
            })
        {
            self.focused_terminal = None;
        }
        if self
            .reported_terminal_focus
            .is_some_and(|(connection_key, pane_id)| {
                connection_key == key && !pane_ids.contains(&pane_id)
            })
        {
            self.reported_terminal_focus = None;
        }
    }

    /// The user selected this Pane: drop its unseen-completion marker.
    pub(super) fn mark_pane_seen(&mut self, key: ConnectionKey, pane_id: PaneId) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        if let Some(tracker) = connection.agent_trackers.get_mut(&pane_id) {
            tracker.mark_seen();
        }
    }

    pub(super) fn clear_pane_attention(&mut self, key: ConnectionKey, pane_id: PaneId) -> bool {
        self.connection_mut(key)
            .is_some_and(|connection| connection.attention.remove(&pane_id))
    }

    pub(super) fn connection_focused_pane(&self, key: ConnectionKey) -> Option<PaneId> {
        self.connection(key)
            .and_then(|connection| Session::restore(connection.snapshot.clone()).ok())
            .and_then(|session| {
                Some(
                    session
                        .active_workspace()?
                        .active_tab()
                        .focused_pane()?
                        .id(),
                )
            })
    }

    pub(super) fn select_server(
        &mut self,
        key: ConnectionKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self
            .connection(key)
            .and_then(|connection| Session::restore(connection.snapshot.clone()).ok())
        else {
            return;
        };
        if self.pending_workspace_selection_for(key).is_some() {
            let workspace_id = self
                .active_dock_surface
                .filter(|surface| surface.connection_key == key)
                .and_then(|surface| Self::workspace_id_for_surface(&session, surface))
                .or_else(|| session.active_workspace_id());
            if let Some(workspace_id) = workspace_id {
                self.select_workspace(key, workspace_id, window, cx);
                return;
            }
        }
        if self.active_connection == key {
            self.cancel_pending_presentation(window, cx);
            return;
        }
        self.pending_presentation_request = None;
        self.active_connection = key;
        self.refresh_target_pane(key);
        self.rebuild_dock(window, cx);
        cx.notify();
    }

    pub(super) fn select_workspace(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let active_surface = self.active_dock_surface;
        let Some((
            active_workspace_id,
            displayed_workspace_id,
            workspace_exists,
            can_mutate,
            mut pending,
        )) = self.connection(key).and_then(|connection| {
            let session = Session::restore(connection.snapshot.clone()).ok()?;
            let (Some(server_id), Some(runtime_epoch), Some(session_id)) = (
                connection.server_id,
                connection.runtime_epoch,
                connection.session_id,
            ) else {
                return None;
            };
            Some((
                session.active_workspace_id(),
                active_surface
                    .filter(|surface| surface.connection_key == key)
                    .and_then(|surface| Self::workspace_id_for_surface(&session, surface)),
                session.workspace(workspace_id).is_some(),
                connection.can_mutate(),
                PendingWorkspaceSelection {
                    connection_key: key,
                    workspace_id,
                    pane_id: None,
                    connect_generation: connection.connect_generation,
                    server_id,
                    runtime_epoch,
                    session_id,
                    request_id: 0,
                    applied_sequence: None,
                },
            ))
        })
        else {
            return;
        };
        if !workspace_exists || !can_mutate {
            return;
        }
        if let Some(existing) = self
            .pending_workspace_selection_for(key)
            .filter(|existing| existing.workspace_id == workspace_id && existing.pane_id.is_none())
        {
            self.pending_presentation_request = Some((key, existing.request_id));
            return;
        }
        if self.active_connection == key
            && displayed_workspace_id == Some(workspace_id)
            && active_workspace_id == Some(workspace_id)
            && self.pending_workspace_selection_for(key).is_none()
        {
            self.cancel_pending_presentation(window, cx);
            return;
        }
        if active_workspace_id == Some(workspace_id)
            && self.pending_workspace_selection_for(key).is_none()
        {
            self.pending_presentation_request = None;
            self.active_connection = key;
            self.refresh_target_pane(key);
            self.rebuild_dock(window, cx);
            cx.notify();
            return;
        }

        let Some(request_id) =
            self.send_layout_to(key, LayoutCommand::ActivateWorkspace { workspace_id })
        else {
            return;
        };
        pending.request_id = request_id;
        self.pending_workspace_selections.insert(key, pending);
        self.pending_presentation_request = Some((key, request_id));
    }

    pub(crate) fn select_pane(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((workspace_id, tab_id, authoritative_target, can_mutate, mut pending)) =
            self.connection(key).and_then(|connection| {
                let session = Session::restore(connection.snapshot.clone()).ok()?;
                let (workspace_id, tab_id) = session.workspaces().iter().find_map(|workspace| {
                    workspace.tabs().iter().find_map(|tab| {
                        tab.panes()
                            .iter()
                            .any(|pane| pane.id() == pane_id)
                            .then_some((workspace.id(), tab.id()))
                    })
                })?;
                let authoritative_target = session.active_workspace().is_some_and(|workspace| {
                    workspace.id() == workspace_id
                        && workspace.active_tab().id() == tab_id
                        && workspace
                            .active_tab()
                            .focused_pane()
                            .is_some_and(|pane| pane.id() == pane_id)
                });
                let (Some(server_id), Some(runtime_epoch), Some(session_id)) = (
                    connection.server_id,
                    connection.runtime_epoch,
                    connection.session_id,
                ) else {
                    return None;
                };
                Some((
                    workspace_id,
                    tab_id,
                    authoritative_target,
                    connection.can_mutate(),
                    PendingWorkspaceSelection {
                        connection_key: key,
                        workspace_id,
                        pane_id: Some(pane_id),
                        connect_generation: connection.connect_generation,
                        server_id,
                        runtime_epoch,
                        session_id,
                        request_id: 0,
                        applied_sequence: None,
                    },
                ))
            })
        else {
            return false;
        };
        let target_surface = DockSurfaceKey {
            connection_key: key,
            tab_id,
        };
        let target_is_displayed =
            self.active_connection == key && self.active_dock_surface == Some(target_surface);
        if !can_mutate {
            if target_is_displayed {
                self.target_pane = Some((key, pane_id));
                self.mark_pane_seen(key, pane_id);
                self.focus_pane_panel(target_surface, pane_id, window, cx);
                cx.notify();
                return true;
            }
            return false;
        }
        if let Some(selection) = self
            .pending_workspace_selection_for(key)
            .filter(|selection| {
                selection.workspace_id == workspace_id && selection.pane_id == Some(pane_id)
            })
        {
            self.pending_presentation_request = Some((key, selection.request_id));
            return true;
        }

        let supersedes_same_connection = self.pending_workspace_selection_for(key).is_some();
        if authoritative_target && !supersedes_same_connection {
            self.pending_presentation_request = None;
            self.active_connection = key;
            self.target_pane = Some((key, pane_id));
            self.mark_pane_seen(key, pane_id);
            self.rebuild_dock(window, cx);
            cx.notify();
            return true;
        }

        let Some(request_id) = self.send_layout_to(key, LayoutCommand::FocusPane { pane_id })
        else {
            return false;
        };
        pending.request_id = request_id;
        self.pending_workspace_selections.insert(key, pending);
        self.pending_presentation_request = Some((key, request_id));
        self.mark_pane_seen(key, pane_id);
        if target_is_displayed {
            self.target_pane = Some((key, pane_id));
            self.focus_pane_panel(target_surface, pane_id, window, cx);
        }
        cx.notify();
        true
    }

    pub(super) fn set_target_pane(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> Option<bool> {
        if self.pending_workspace_selection_for(key).is_some() {
            return None;
        }
        if !self
            .connection(key)
            .is_some_and(ServerConnection::can_mutate)
        {
            return None;
        }
        let changed = self.active_connection != key || self.target_pane != Some((key, pane_id));
        self.active_connection = key;
        self.target_pane = Some((key, pane_id));
        self.mark_pane_seen(key, pane_id);
        if changed {
            cx.notify();
        }
        Some(changed)
    }

    pub(super) fn send_layout(&mut self, command: LayoutCommand) {
        self.send_layout_to(self.active_connection, command);
    }

    pub(super) fn activate_tab_on(
        &mut self,
        key: ConnectionKey,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .send_layout_to(key, LayoutCommand::ActivateTab { tab_id })
            .is_some()
        {
            self.cancel_pending_presentation(window, cx);
        }
    }

    pub(super) fn send_presenting_layout_to(
        &mut self,
        key: ConnectionKey,
        command: LayoutCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let request_id = self.send_layout_to(key, command)?;
        self.pending_presentation_request = None;
        self.active_connection = key;
        self.target_pane = None;
        self.rebuild_dock(window, cx);
        cx.notify();
        Some(request_id)
    }

    pub(super) fn send_layout_to(
        &mut self,
        key: ConnectionKey,
        command: LayoutCommand,
    ) -> Option<u64> {
        if (self.has_pending_projection_for(key)
            || self.pending_workspace_selection_for(key).is_some())
            && !matches!(
                &command,
                LayoutCommand::ActivateWorkspace { .. }
                    | LayoutCommand::FocusPane { .. }
                    | LayoutCommand::SetSplitRatios { .. }
            )
        {
            return None;
        }
        let connection = self.connection_mut(key)?;
        let (Some(server_id), Some(session_id)) = (connection.server_id, connection.session_id)
        else {
            return None;
        };
        if connection.can_mutate() {
            let request_id = connection.next_layout_request_id;
            connection.next_layout_request_id = request_id.wrapping_add(1).max(1);
            connection.send(ClientMessage::Layout {
                server_id,
                session_id,
                request_id,
                command,
            });
            (connection.status == ConnectionStatus::Connected).then_some(request_id)
        } else {
            None
        }
    }

    pub(super) fn terminal_command(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        command: TerminalCommand,
    ) -> bool {
        let Some(connection) = self.connection_mut(key) else {
            return false;
        };
        let (Some(server_id), Some(session_id)) = (connection.server_id, connection.session_id)
        else {
            return false;
        };
        let read_only = matches!(&command, TerminalCommand::Copy { .. });
        let synchronizes_focus = matches!(&command, TerminalCommand::Focus(_));
        if ((synchronizes_focus && connection.controlling)
            || (read_only && connection.is_synchronized())
            || connection.can_mutate())
            && (synchronizes_focus
                || !connection
                    .terminals
                    .get(&pane_id)
                    .is_some_and(|terminal| terminal.exited))
        {
            connection.send(ClientMessage::Terminal {
                server_id,
                session_id,
                pane_id,
                command,
            });
            connection.status == ConnectionStatus::Connected
        } else {
            false
        }
    }

    pub(super) fn reconnect_active(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.connect_server(self.active_connection, window, cx);
    }

    pub(super) fn new_workspace_on(
        &mut self,
        key: ConnectionKey,
        root_directory: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_presenting_layout_to(
            key,
            LayoutCommand::CreateWorkspace {
                root_directory,
                name: None,
                focus: true,
            },
            window,
            cx,
        );
    }

    pub(super) fn choose_workspace_directory_on(
        &mut self,
        key: ConnectionKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(label) = self
            .connection(key)
            .map(|connection| connection.label.clone())
        else {
            return;
        };
        self.prompt_directory_on(
            key,
            format!("New Workspace on {label}"),
            "Create",
            move |this, path, window, cx| this.new_workspace_on(key, path, window, cx),
            window,
            cx,
        );
    }

    pub(super) fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace_id) = self
            .active_session()
            .and_then(|session| session.active_workspace_id())
        else {
            return;
        };
        self.new_tab_on(self.active_connection, workspace_id, window, cx);
    }

    pub(super) fn new_tab_on(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_presenting_layout_to(
            key,
            LayoutCommand::CreateTab {
                workspace_id,
                name: None,
                focus: true,
            },
            window,
            cx,
        );
    }

    pub(super) fn activate_tab_at_index(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.active_session() else {
            return;
        };
        let key = self.active_connection;
        let Some(workspace) = self
            .presented_workspace_id(key, &session)
            .and_then(|id| session.workspace(id))
        else {
            return;
        };
        if let Some(tab) = workspace.tabs().get(index) {
            self.activate_tab_on(key, tab.id(), window, cx);
        }
    }

    pub(super) fn cycle_tab(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.active_session() else {
            return;
        };
        let Some(workspace) = session.active_workspace() else {
            return;
        };
        let tabs = workspace.tabs();
        let Some(active_ix) = tabs
            .iter()
            .position(|tab| tab.id() == workspace.active_tab().id())
        else {
            return;
        };
        let target_ix = (active_ix as isize + step).rem_euclid(tabs.len() as isize) as usize;
        self.activate_tab_on(self.active_connection, tabs[target_ix].id(), window, cx);
    }

    pub(super) fn split(&mut self, direction: SplitDirection) {
        if let Some(pane_id) = self.focused_pane() {
            self.send_layout(LayoutCommand::SplitPane {
                pane_id,
                direction,
                focus: true,
            });
        }
    }

    pub(super) fn focus_direction(&mut self, direction: PaneDirection) {
        if let Some(pane_id) = self.focused_pane() {
            self.send_layout(LayoutCommand::FocusPaneDirection { pane_id, direction });
        }
    }

    pub(super) fn resize_direction(&mut self, direction: PaneDirection) {
        if let Some(pane_id) = self.focused_pane() {
            self.send_layout(LayoutCommand::ResizePane {
                pane_id,
                direction,
                amount: 0.05,
            });
        }
    }

    pub(super) fn swap_direction(&mut self, direction: PaneDirection) {
        if let Some(pane_id) = self.focused_pane() {
            self.send_layout(LayoutCommand::SwapPane { pane_id, direction });
        }
    }

    pub(super) fn toggle_zoom(&mut self) {
        if let Some(pane_id) = self.focused_pane() {
            self.send_layout(LayoutCommand::TogglePaneZoom { pane_id });
        }
    }

    pub(super) fn close_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane_id) = self.focused_pane() else {
            return;
        };
        let closes_workspace = self.active_session().is_some_and(|session| {
            session.workspaces().iter().any(|workspace| {
                workspace.tabs().len() == 1
                    && workspace.tabs()[0].panes().len() == 1
                    && workspace.tabs()[0].panes()[0].id() == pane_id
            })
        });
        self.confirm_close_on(
            self.active_connection,
            LayoutCommand::ClosePane { pane_id },
            closes_workspace,
            window,
            cx,
        );
    }

    pub(super) fn close_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.active_session() else {
            return;
        };
        let Some(workspace) = session.active_workspace() else {
            return;
        };
        self.close_tab_id(
            self.active_connection,
            workspace.active_tab().id(),
            workspace.tabs().len() == 1,
            window,
            cx,
        );
    }

    pub(super) fn close_tab_id(
        &mut self,
        key: ConnectionKey,
        tab_id: TabId,
        closes_workspace: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_close_on(
            key,
            LayoutCommand::CloseTab { tab_id },
            closes_workspace,
            window,
            cx,
        );
    }

    pub(super) fn close_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace_id) = self
            .active_session()
            .and_then(|session| session.active_workspace_id())
        else {
            return;
        };
        self.close_workspace_id(self.active_connection, workspace_id, window, cx);
    }

    pub(super) fn close_workspace_id(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.confirm_close_on(
            key,
            LayoutCommand::CloseWorkspace { workspace_id },
            true,
            window,
            cx,
        );
    }

    pub(super) fn confirm_close_on(
        &mut self,
        key: ConnectionKey,
        command: LayoutCommand,
        closes_workspace: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !closes_workspace {
            self.send_layout_to(key, command);
            return;
        }
        let owner = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            window.open_alert_dialog(cx, move |alert, _, _| {
                let owner = owner.clone();
                let command = command.clone();
                alert
                    .confirm()
                    .title("Close Workspace?")
                    .description("The workspace terminals will be stopped. Files are not deleted.")
                    .on_ok(move |_, _, cx| {
                        let _ =
                            owner.update(cx, |this, _| this.send_layout_to(key, command.clone()));
                        true
                    })
            });
        });
    }
}
