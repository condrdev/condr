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
        self.prune_files_state(key, &session);

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

    /// The focused Pane of the Tab connection `key` shows.
    pub(super) fn connection_focused_pane(&self, key: ConnectionKey) -> Option<PaneId> {
        let connection = self.connection(key)?;
        let session = connection.session()?;
        let (_, tab_id) = connection.viewed(&session)?;
        Some(session.tab(tab_id)?.focused_pane()?.id())
    }

    pub(super) fn select_server(
        &mut self,
        key: ConnectionKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.connection(key).is_none() || self.active_connection == key {
            return;
        }
        self.active_connection = key;
        self.refresh_target_pane(key);
        self.rebuild_dock(window, cx);
        cx.notify();
    }

    /// Shows a Workspace. The view is this Client's own (ADR 0021): nothing goes to the
    /// Server, and a viewing-only connection can browse too.
    pub(super) fn select_workspace(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.show_view(key, workspace_id, None) {
            self.rebuild_dock(window, cx);
        }
        cx.notify();
    }

    /// Shows a Pane's Tab and targets the Pane. Pane focus inside a Tab is Session
    /// structure, so a connection that may mutate also tells the Server.
    pub(crate) fn select_pane(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((workspace_id, tab_id, server_focused, can_mutate)) =
            self.connection(key).and_then(|connection| {
                let session = connection.session()?;
                let workspace = session.workspace_for_pane(pane_id)?;
                let tab = workspace
                    .tabs()
                    .iter()
                    .find(|tab| tab.panes().iter().any(|pane| pane.id() == pane_id))?;
                Some((
                    workspace.id(),
                    tab.id(),
                    tab.focused_pane().is_some_and(|pane| pane.id() == pane_id),
                    connection.can_mutate(),
                ))
            })
        else {
            return false;
        };
        self.show_view(key, workspace_id, Some(tab_id));
        self.target_pane = Some((key, pane_id));
        self.mark_pane_seen(key, pane_id);
        if can_mutate && !server_focused {
            self.send_layout_to(key, LayoutCommand::FocusPane { pane_id });
        }
        self.rebuild_dock(window, cx);
        self.focus_pane_panel(
            DockSurfaceKey {
                connection_key: key,
                tab_id,
            },
            pane_id,
            window,
            cx,
        );
        cx.notify();
        true
    }

    pub(super) fn set_target_pane(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> Option<bool> {
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

    /// Shows a Tab; local, like `select_workspace`.
    pub(super) fn activate_tab_on(
        &mut self,
        key: ConnectionKey,
        tab_id: TabId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(workspace_id) = self
            .connection(key)
            .and_then(ServerConnection::session)
            .and_then(|session| {
                session
                    .workspaces()
                    .iter()
                    .find(|workspace| workspace.tab(tab_id).is_some())
                    .map(Workspace::id)
            })
        else {
            return;
        };
        if self.show_view(key, workspace_id, Some(tab_id)) {
            self.rebuild_dock(window, cx);
        }
        cx.notify();
    }

    /// Sends a command whose result this Client will show: the created Workspace or Tab,
    /// the shown diff or file. The view moves when `LayoutApplied` names it.
    pub(super) fn send_presenting_layout_to(
        &mut self,
        key: ConnectionKey,
        command: LayoutCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<u64> {
        let request_id = self.send_layout_to(key, command)?;
        if self.active_connection != key {
            self.active_connection = key;
            self.refresh_target_pane(key);
            self.rebuild_dock(window, cx);
        }
        cx.notify();
        Some(request_id)
    }

    pub(super) fn send_layout_to(
        &mut self,
        key: ConnectionKey,
        command: LayoutCommand,
    ) -> Option<u64> {
        if self.has_pending_projection_for(key)
            && !matches!(
                &command,
                LayoutCommand::FocusPane { .. } | LayoutCommand::SetSplitRatios { .. }
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
        let Some((key, _, workspace_id, _)) = self.presented() else {
            return;
        };
        self.new_tab_on(key, workspace_id, window, cx);
    }

    /// The new Tab's shell starts where the Pane this Client shows in that Workspace is.
    pub(super) fn new_tab_on(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cwd_from = self.connection(key).and_then(|connection| {
            let session = connection.session()?;
            let tab_id = connection.viewed_tab_id(&session, workspace_id)?;
            Some(session.tab(tab_id)?.focused_pane()?.id())
        });
        self.send_presenting_layout_to(
            key,
            LayoutCommand::CreateTab {
                workspace_id,
                name: None,
                cwd_from,
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
        let Some((key, session, workspace_id, _)) = self.presented() else {
            return;
        };
        let Some(tab) = session
            .workspace(workspace_id)
            .and_then(|workspace| workspace.tabs().get(index))
        else {
            return;
        };
        self.activate_tab_on(key, tab.id(), window, cx);
    }

    pub(super) fn cycle_tab(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let Some((key, session, workspace_id, tab_id)) = self.presented() else {
            return;
        };
        let Some(workspace) = session.workspace(workspace_id) else {
            return;
        };
        let tabs = workspace.tabs();
        let Some(active_ix) = tabs.iter().position(|tab| tab.id() == tab_id) else {
            return;
        };
        let target_ix = (active_ix as isize + step).rem_euclid(tabs.len() as isize) as usize;
        self.activate_tab_on(key, tabs[target_ix].id(), window, cx);
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
        let Some((key, session, workspace_id, tab_id)) = self.presented() else {
            return;
        };
        let Some(workspace) = session.workspace(workspace_id) else {
            return;
        };
        self.close_tab_id(key, tab_id, workspace.tabs().len() == 1, window, cx);
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
        let Some((key, _, workspace_id, _)) = self.presented() else {
            return;
        };
        self.close_workspace_id(key, workspace_id, window, cx);
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
