use super::*;

impl Condr {
    pub(super) fn install_connection(
        connection: &mut ServerConnection,
        result: Result<ClientConnection, String>,
        window: &Window,
        cx: &Context<Self>,
    ) -> Result<BootstrapApplication, String> {
        let client = result?;
        let bootstrap = client.bootstrap().clone();
        let io = ClientIo::start(
            client,
            connection.key,
            connection.connect_generation,
            bootstrap.server_id,
            bootstrap.session_id,
            window,
            cx,
        )
        .map_err(|error| error.to_string())?;
        let application = connection.apply_bootstrap(bootstrap);
        connection.io = Some(io);
        connection.reset_sync_state();
        Ok(application)
    }

    pub(super) fn connection(&self, key: ConnectionKey) -> Option<&ServerConnection> {
        self.connections
            .iter()
            .find(|connection| connection.key == key)
    }

    pub(super) fn connection_mut(&mut self, key: ConnectionKey) -> Option<&mut ServerConnection> {
        self.connections
            .iter_mut()
            .find(|connection| connection.key == key)
    }

    pub(super) fn active_connection(&self) -> Option<&ServerConnection> {
        self.connection(self.active_connection)
    }

    pub(super) fn active_session(&self) -> Option<Session> {
        let connection = self.active_connection()?;
        let mut session = Session::restore(connection.snapshot.clone()).ok()?;
        if let Some(surface) = self
            .active_dock_surface
            .filter(|surface| surface.connection_key == connection.key)
        {
            session.activate_tab(surface.tab_id);
        }
        Some(session)
    }

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

    pub(crate) fn terminal(
        &self,
        connection_key: ConnectionKey,
        pane_id: PaneId,
    ) -> Option<&ClientTerminal> {
        self.connection(connection_key)?.terminals.get(&pane_id)
    }

    pub(super) fn acquire_and_subscribe(&mut self, key: ConnectionKey) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let Some(session_id) = connection.session_id else {
            return;
        };
        connection.send(ClientMessage::AcquireControl { session_id });
        connection.subscribe();
    }

    pub(super) fn schedule_control_retry(&mut self, key: ConnectionKey, cx: &mut Context<Self>) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        if connection.status != ConnectionStatus::Connected
            || connection.controlling
            || connection.control_retry_scheduled
        {
            return;
        }
        let Some(session_id) = connection.session_id else {
            return;
        };
        let generation = connection.connect_generation;
        connection.control_retry_scheduled = true;
        let delay = CONTROL_RETRY_DELAY
            .saturating_mul(1 << connection.control_retry_attempts.min(8))
            .min(MAX_CONTROL_RETRY_DELAY);
        connection.control_retry_attempts = connection.control_retry_attempts.saturating_add(1);

        cx.spawn(async move |owner, cx| {
            cx.background_executor().timer(delay).await;
            owner
                .update(cx, |this, cx| {
                    let Some(connection) = this.connection_mut(key) else {
                        return;
                    };
                    if connection.connect_generation != generation {
                        return;
                    }
                    connection.control_retry_scheduled = false;
                    if connection.status == ConnectionStatus::Connected
                        && !connection.controlling
                        && connection.session_id == Some(session_id)
                    {
                        connection.send(ClientMessage::AcquireControl { session_id });
                        cx.notify();
                    }
                })
                .ok();
        })
        .detach();
    }

    pub(super) fn start_connect(&mut self, key: ConnectionKey) -> bool {
        let was_holding = self.should_hold_active_surface();
        self.clear_pending_workspace_selection_for(key);
        let active_projection_cleared = self.clear_pending_projections_for(key);
        let needs_active_rebuild =
            active_projection_cleared || (was_holding && !self.should_hold_active_surface());
        let Some(connection) = self.connection_mut(key) else {
            return needs_active_rebuild;
        };
        if connection.status == ConnectionStatus::Connecting {
            return needs_active_rebuild;
        }
        if let Some(io) = connection.io.take() {
            let _ = io.outgoing.send(ClientMessage::Detach);
        }
        connection.connect_generation = connection.connect_generation.wrapping_add(1);
        let generation = connection.connect_generation;
        connection.status = ConnectionStatus::Connecting;
        connection.reset_sync_state();
        connection.error = None;
        let endpoint = connection.endpoint.clone();
        let sender = self.connect_results_tx.clone();
        clear_pending_sizes_for_bootstrap(&mut self.pending_sizes, key);
        thread::spawn(move || {
            let (connected_endpoint, result) = connect_to_server(endpoint, "condr");
            let _ = sender.send_blocking(ConnectionResult {
                key,
                generation,
                endpoint: connected_endpoint,
                result,
            });
        });
        needs_active_rebuild
    }

    pub(super) fn disconnect_server(&mut self, key: ConnectionKey) -> bool {
        let was_holding = self.should_hold_active_surface();
        let active_projection_cleared = self.clear_pending_projections_for(key);
        let Some(connection) = self.connection_mut(key) else {
            return active_projection_cleared;
        };
        if let Some(io) = connection.io.take() {
            let _ = io.outgoing.send(ClientMessage::Detach);
        }
        connection.connect_generation = connection.connect_generation.wrapping_add(1);
        connection.status = ConnectionStatus::Disconnected;
        connection.reset_sync_state();
        connection.error = None;
        self.clear_pending_workspace_selection_for(key);
        clear_pending_sizes_for_bootstrap(&mut self.pending_sizes, key);
        active_projection_cleared || (was_holding && !self.should_hold_active_surface())
    }

    pub(super) fn remove_server(
        &mut self,
        key: ConnectionKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let released_presentation = self.disconnect_server(key);
        let Some(index) = self
            .connections
            .iter()
            .position(|connection| connection.key == key)
        else {
            return;
        };
        let was_active = self.active_connection == key;
        self.connections.remove(index);
        self.save_servers();
        self.clear_connection_gui_state(key);
        self.clear_pending_workspace_selection_for(key);
        if was_active {
            self.active_connection = self
                .connections
                .get(index)
                .or_else(|| self.connections.last())
                .map_or(0, |connection| connection.key);
            self.refresh_target_pane(self.active_connection);
            self.rebuild_dock(window, cx);
        } else if released_presentation {
            self.refresh_target_pane(self.active_connection);
            self.rebuild_dock(window, cx);
        }
        cx.notify();
    }

    pub(super) fn handle_connection_result(
        &mut self,
        result: ConnectionResult,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ConnectionResult {
            key,
            generation,
            endpoint,
            result,
        } = result;
        if !self.connection(key).is_some_and(|connection| {
            connection.status == ConnectionStatus::Connecting
                && connection.connect_generation == generation
        }) {
            return;
        }
        let application = if let Some(connection) = self.connection_mut(key) {
            connection.endpoint = endpoint;
            match Self::install_connection(connection, result, window, cx) {
                Ok(application) => Some(application),
                Err(error) => {
                    connection.status = ConnectionStatus::Disconnected;
                    connection.error = Some(error);
                    None
                }
            }
        } else {
            None
        };
        if let Some(application) = application {
            let presentation_before = self.pending_presentation_request;
            _ = self.clear_pending_projections_for(key);
            clear_pending_sizes_for_bootstrap(&mut self.pending_sizes, key);
            if application.authority_changed {
                self.clear_connection_gui_state(key);
            } else {
                self.prune_dock_cache(key);
            }
            if application.reacquire_control {
                self.clear_pending_workspace_selection_for(key);
            }
            self.acquire_and_subscribe(key);
            self.refresh_target_pane(key);
            let released_presentation = presentation_before.is_some()
                && self.pending_presentation_request != presentation_before;
            if key == self.active_connection || released_presentation {
                self.rebuild_dock(window, cx);
            }
        }
    }

    pub(super) fn handle_incoming(
        &mut self,
        key: ConnectionKey,
        generation: u64,
        incoming: Incoming,
        cx: &mut Context<Self>,
    ) -> IncomingEffect {
        let Some(index) = self
            .connections
            .iter()
            .position(|connection| connection.key == key)
        else {
            return IncomingEffect::default();
        };
        if self.connections[index].connect_generation != generation {
            return IncomingEffect::default();
        }
        let message = match incoming {
            Incoming::Bootstrap(bootstrap) => {
                let presentation_before = self.pending_presentation_request;
                let application = self.connections[index].apply_bootstrap(bootstrap);
                if self
                    .hovered_link
                    .as_ref()
                    .is_some_and(|(hover_key, _, _)| *hover_key == key)
                {
                    self.hovered_link = None;
                }
                if self
                    .pressed_terminal_link
                    .as_ref()
                    .is_some_and(|(press_key, _, _)| *press_key == key)
                {
                    self.pressed_terminal_link = None;
                }
                clear_pending_sizes_for_bootstrap(&mut self.pending_sizes, key);
                if application.authority_changed {
                    self.clear_connection_gui_state(key);
                } else {
                    self.prune_dock_cache(key);
                }
                if application.reacquire_control {
                    // Layout responses may have been lost to writer lag; this Bootstrap is
                    // the authoritative layout, so nothing stays pending against it.
                    self.clear_pending_workspace_selection_for(key);
                    _ = self.clear_pending_projections_for(key);
                }

                let bootstrap_sequence = self.connections[index].sequence;
                let active_surface = self.active_dock_surface;
                let mut active_projection_resolved = false;
                for (surface_key, surface) in &mut self.dock_surfaces {
                    if surface_key.connection_key == key
                        && surface
                            .pending_projection_applied_sequence
                            .is_some_and(|sequence| sequence <= bootstrap_sequence)
                    {
                        surface.pending_projection_request = None;
                        surface.pending_projection_applied_sequence = None;
                        active_projection_resolved |= active_surface == Some(*surface_key);
                    }
                }
                let session = Session::restore(self.connections[index].snapshot.clone()).ok();
                let mut pending_workspace_ready = false;
                let mut pending_workspace_failed = false;
                if let (Some(pending), Some(session)) =
                    (self.pending_workspace_selection_for(key), session.as_ref())
                    && pending
                        .applied_sequence
                        .is_some_and(|sequence| sequence <= bootstrap_sequence)
                {
                    self.pending_workspace_selections.remove(&key);
                    let should_present =
                        self.pending_presentation_request == Some((key, pending.request_id));
                    if should_present {
                        self.pending_presentation_request = None;
                    }
                    let target_is_active = session.active_workspace_id()
                        == Some(pending.workspace_id)
                        && pending.pane_id.is_none_or(|pane_id| {
                            session.active_workspace().is_some_and(|workspace| {
                                workspace.active_tab().focused_pane().id() == pane_id
                            })
                        });
                    if target_is_active {
                        if should_present {
                            self.active_connection = key;
                        }
                        pending_workspace_ready = self.active_connection == key;
                    } else {
                        pending_workspace_failed = self.active_connection == key;
                    }
                }
                let preserve_visible_workspace = self.active_connection == key
                    && self.should_hold_active_surface()
                    && self
                        .active_dock_surface
                        .is_some_and(|surface| surface.connection_key == key);
                let target_before_refresh = self.target_pane;
                if application.reacquire_control {
                    self.acquire_and_subscribe(key);
                } else if application.resubscribe {
                    self.connections[index].subscribe();
                }
                if !preserve_visible_workspace {
                    self.refresh_target_pane(key);
                }
                let target_changed =
                    self.active_connection == key && self.target_pane != target_before_refresh;
                let released_presentation = presentation_before.is_some()
                    && self.pending_presentation_request != presentation_before;
                return IncomingEffect {
                    rebuild: !preserve_visible_workspace
                        && (application.rebuild
                            || application.reacquire_control
                            || pending_workspace_ready
                            || pending_workspace_failed
                            || active_projection_resolved
                            || target_changed),
                    rebuild_active: released_presentation && self.active_connection != key,
                    notify: true,
                };
            }
            Incoming::Message(message) => message,
            Incoming::TerminalResync => {
                let notify = self.connections[index].request_snapshot();
                return IncomingEffect {
                    notify,
                    ..IncomingEffect::default()
                };
            }
            Incoming::VisualReady(_) => return IncomingEffect::default(),
            Incoming::Disconnected(error) => {
                return self.mark_disconnected(
                    key,
                    index,
                    format!("Server connection closed: {error}"),
                );
            }
        };

        match message {
            ServerMessage::Bootstrap(_)
            | ServerMessage::BootstrapBatch(_)
            | ServerMessage::TerminalFrameChunk(_) => {
                let notify = self.connections[index].request_snapshot();
                IncomingEffect {
                    notify,
                    ..IncomingEffect::default()
                }
            }
            ServerMessage::SnapshotRejected {
                server_id,
                session_id,
                reason,
            } => {
                let notify = self.connections[index]
                    .recover_rejected_snapshot(server_id, session_id, reason);
                IncomingEffect {
                    notify,
                    ..IncomingEffect::default()
                }
            }
            ServerMessage::Event {
                server_id,
                session_id,
                sequence,
                event,
            } => {
                let connection = &mut self.connections[index];
                if connection.server_id != Some(server_id)
                    || connection.session_id != Some(session_id)
                {
                    return IncomingEffect::default();
                }
                if sequence != connection.sequence.saturating_add(1) {
                    if sequence > connection.sequence {
                        connection.subscribed = false;
                        connection.subscription_pending = false;
                        connection.request_snapshot();
                        return IncomingEffect {
                            notify: true,
                            ..IncomingEffect::default()
                        };
                    }
                    return IncomingEffect::default();
                }
                self.connections[index].sequence = sequence;
                let notify;
                match event {
                    SessionEvent::LayoutChanged => {
                        notify = self.connections[index].request_snapshot();
                    }
                    SessionEvent::TerminalExited { pane_id } => {
                        if let Some(terminal) = self.connections[index].terminals.get_mut(&pane_id)
                        {
                            terminal.exited = true;
                            notify = true;
                        } else {
                            notify = self.connections[index].request_snapshot();
                        }
                    }
                    SessionEvent::AgentChanged { pane_id, agent } => {
                        // `focused_terminal` is None while the window is inactive, so a
                        // completion behind another window still shows as done.
                        let visible = self.focused_terminal == Some((key, pane_id));
                        if let Some(agent) = agent {
                            self.connections[index].agents.insert(pane_id, agent);
                            self.connections[index]
                                .agent_trackers
                                .entry(pane_id)
                                .and_modify(|tracker| {
                                    // Unknown is only published for a new process; it
                                    // must not carry the previous generation's done.
                                    if agent.state == AgentState::Unknown {
                                        *tracker = AgentTracker::new(agent.state);
                                    } else {
                                        tracker.update(agent.state, visible);
                                    }
                                })
                                .or_insert_with(|| AgentTracker::new(agent.state));
                        } else {
                            self.connections[index].agents.remove(&pane_id);
                            self.connections[index].agent_trackers.remove(&pane_id);
                        }
                        notify = true;
                    }
                    SessionEvent::WorkspaceGitChanged { workspace_id, git } => {
                        if let Some(git) = git {
                            self.connections[index]
                                .workspace_git
                                .insert(workspace_id, git);
                        } else {
                            self.connections[index].workspace_git.remove(&workspace_id);
                        }
                        notify = true;
                    }
                    SessionEvent::TerminalTitleChanged { pane_id, title } => {
                        // An unknown Pane's title arrives with its Bootstrap record instead.
                        notify = match self.connections[index].terminals.get_mut(&pane_id) {
                            Some(terminal) => {
                                terminal.title = title;
                                true
                            }
                            None => false,
                        };
                    }
                    SessionEvent::TerminalAttentionChanged { pane_id, attention } => {
                        let focused = self.focused_terminal == Some((key, pane_id));
                        notify = if attention {
                            !focused && self.connections[index].attention.insert(pane_id)
                        } else {
                            self.connections[index].attention.remove(&pane_id)
                        };
                    }
                    SessionEvent::ServerSettingsChanged { settings } => {
                        self.connections[index].settings = settings;
                        notify = true;
                    }
                }
                IncomingEffect {
                    notify,
                    ..IncomingEffect::default()
                }
            }
            ServerMessage::TerminalFrame(batch) => {
                if self.connections[index].server_id != Some(batch.server_id)
                    || self.connections[index].session_id != Some(batch.session_id)
                {
                    return IncomingEffect::default();
                }

                let connection = &mut self.connections[index];
                let pane_ids = match apply_terminal_frame_batch(
                    &mut connection.terminals,
                    &mut connection.terminal_hyperlinks,
                    batch.panes,
                ) {
                    Ok(pane_ids) => pane_ids,
                    Err(()) => {
                        let notify = self.connections[index].request_snapshot();
                        return IncomingEffect {
                            notify,
                            ..IncomingEffect::default()
                        };
                    }
                };
                if let Some(selection) = &mut self.terminal_selection
                    && selection.connection_key == key
                    && pane_ids.contains(&selection.pane_id)
                {
                    let server_selection = self.connections[index].terminals[&selection.pane_id]
                        .view
                        .selection;
                    if selection.committed && server_selection.is_some() {
                        // The Server's frame now carries this selection; a frame that was
                        // already in flight before the Select keeps the local bridge.
                        self.terminal_selection = None;
                    } else if !selection.committed {
                        selection.range.display_offset = self.connections[index].terminals
                            [&selection.pane_id]
                            .view
                            .display_offset;
                    }
                }
                if self.last_terminal_mouse_motion.is_some_and(|motion| {
                    motion.connection_key == key
                        && pane_ids.contains(&motion.pane_id)
                        && self.connections[index]
                            .terminals
                            .get(&motion.pane_id)
                            .is_some_and(|terminal| {
                                terminal.view.mouse_tracking != motion.mouse_tracking
                            })
                }) {
                    self.last_terminal_mouse_motion = None;
                }
                if let Some((hover_key, hover_pane, hovered)) = self.hovered_link.take() {
                    self.hovered_link = if hover_key == key && pane_ids.contains(&hover_pane) {
                        self.connections[index]
                            .terminals
                            .get(&hover_pane)
                            .and_then(|terminal| {
                                link_at(
                                    &terminal.view,
                                    hovered.position.row,
                                    hovered.position.column,
                                )
                            })
                            .map(|link| (hover_key, hover_pane, link))
                    } else {
                        Some((hover_key, hover_pane, hovered))
                    };
                }
                for pane_id in &pane_ids {
                    let terminal_size = self.connections[index]
                        .terminals
                        .get(pane_id)
                        .expect("applied terminal still exists")
                        .view
                        .size;
                    let pending_key = (key, *pane_id);
                    if self.pending_sizes.get(&pending_key) == Some(&terminal_size) {
                        self.pending_sizes.remove(&pending_key);
                    }
                }
                // A visual frame redraws only the Panels it touched; re-rendering the
                // root would rebuild the sidebar and every Pane's chrome per frame.
                for pane_id in &pane_ids {
                    if let Some(panel) = self.panels.get(&(key, *pane_id)) {
                        panel.update(cx, |_, cx| cx.notify());
                    }
                }
                IncomingEffect::default()
            }
            ServerMessage::ControlGranted {
                server_id,
                session_id,
            } => {
                if self.connections[index].server_id != Some(server_id)
                    || self.connections[index].session_id != Some(session_id)
                {
                    return IncomingEffect::default();
                }
                self.connections[index].controlling = true;
                self.connections[index].control_retry_attempts = 0;
                self.connections[index].control_retry_scheduled = false;
                self.connections[index].error = None;
                IncomingEffect {
                    notify: true,
                    ..IncomingEffect::default()
                }
            }
            ServerMessage::ControlReleased {
                server_id,
                session_id,
            } => {
                if self.connections[index].server_id != Some(server_id)
                    || self.connections[index].session_id != Some(session_id)
                {
                    return IncomingEffect::default();
                }
                self.connections[index].controlling = false;
                self.connections[index].attention.clear();
                self.connections[index].control_retry_attempts = 0;
                self.connections[index].control_retry_scheduled = false;
                IncomingEffect {
                    notify: true,
                    ..IncomingEffect::default()
                }
            }
            ServerMessage::ControlDenied {
                server_id,
                session_id,
                reason,
            } => {
                if self.connections[index].server_id != Some(server_id)
                    || self.connections[index].session_id != Some(session_id)
                {
                    return IncomingEffect::default();
                }
                let retry_control = reason == CONTROL_BUSY_REASON;
                self.connections[index].controlling = false;
                self.connections[index].attention.clear();
                self.connections[index].error = Some(reason);
                if retry_control {
                    self.schedule_control_retry(key, cx);
                }
                IncomingEffect {
                    notify: true,
                    ..IncomingEffect::default()
                }
            }
            ServerMessage::SubscriptionRejected {
                server_id,
                session_id,
                reason,
            } => {
                let connection = &mut self.connections[index];
                if !connection.recover_rejected_subscription(server_id, session_id) {
                    return IncomingEffect::default();
                }
                connection.error = Some(reason);
                IncomingEffect {
                    notify: true,
                    ..IncomingEffect::default()
                }
            }
            ServerMessage::LayoutApplied {
                server_id,
                session_id,
                request_id,
                sequence,
            } => {
                if self.connections[index].server_id != Some(server_id)
                    || self.connections[index].session_id != Some(session_id)
                {
                    return IncomingEffect::default();
                }
                if let Some(mut pending) = self.pending_workspace_selection_for(key)
                    && pending.request_id == request_id
                {
                    pending.applied_sequence = Some(sequence);
                    self.pending_workspace_selections.insert(key, pending);
                }
                for (surface_key, surface) in &mut self.dock_surfaces {
                    if surface_key.connection_key == key
                        && surface.pending_projection_request == Some(request_id)
                    {
                        surface.pending_projection_applied_sequence = Some(sequence);
                    }
                }
                IncomingEffect::default()
            }
            ServerMessage::LayoutRejected {
                server_id,
                session_id,
                request_id,
                reason,
            } => {
                if self.connections[index].server_id != Some(server_id)
                    || self.connections[index].session_id != Some(session_id)
                {
                    return IncomingEffect::default();
                }
                self.connections[index].error = Some(reason);
                let workspace_rejected = self
                    .pending_workspace_selection_for(key)
                    .is_some_and(|pending| pending.request_id == request_id);
                let presentation_rejected =
                    self.pending_presentation_request == Some((key, request_id));
                if workspace_rejected {
                    self.pending_workspace_selections.remove(&key);
                    if presentation_rejected {
                        self.pending_presentation_request = None;
                    }
                }
                let mut active_projection_rejected = false;
                for (surface_key, surface) in &mut self.dock_surfaces {
                    if surface_key.connection_key == key
                        && surface.pending_projection_request == Some(request_id)
                    {
                        surface.pending_projection_request = None;
                        surface.pending_projection_applied_sequence = None;
                        surface.projection = None;
                        active_projection_rejected |=
                            self.active_dock_surface == Some(*surface_key);
                    }
                }
                IncomingEffect {
                    rebuild: self.active_connection == key
                        && (workspace_rejected || active_projection_rejected),
                    rebuild_active: presentation_rejected,
                    notify: true,
                }
            }
            ServerMessage::Error { message } => {
                self.connections[index].error = Some(message);
                IncomingEffect {
                    notify: true,
                    ..IncomingEffect::default()
                }
            }
            ServerMessage::TerminalCopied { text, .. } => {
                if let Some(text) = text {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                IncomingEffect::default()
            }
            ServerMessage::TerminalClipboard { text, .. } => {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
                IncomingEffect::default()
            }
            ServerMessage::ServerStopping => {
                self.mark_disconnected(key, index, "condr-server stopped".into())
            }
            ServerMessage::Subscribed {
                server_id,
                session_id,
                sequence,
            } => {
                let connection = &mut self.connections[index];
                if !connection.subscription_pending {
                    return IncomingEffect::default();
                }
                connection.subscription_pending = false;
                if connection.server_id == Some(server_id)
                    && connection.session_id == Some(session_id)
                    && connection.sequence == sequence
                {
                    connection.subscribed = true;
                } else {
                    connection.subscribed = false;
                    connection.error =
                        Some("subscription cursor did not match client state".into());
                    connection.request_snapshot();
                }
                IncomingEffect {
                    notify: true,
                    ..IncomingEffect::default()
                }
            }
            ServerMessage::Welcome { .. } | ServerMessage::Pong { .. } => IncomingEffect::default(),
        }
    }

    fn mark_disconnected(
        &mut self,
        key: ConnectionKey,
        index: usize,
        message: String,
    ) -> IncomingEffect {
        let connection = &mut self.connections[index];
        connection.status = ConnectionStatus::Disconnected;
        connection.reset_sync_state();
        connection.error = Some(message);
        connection.io = None;
        let active_projection_cleared = self.clear_pending_projections_for(key);
        clear_pending_sizes_for_bootstrap(&mut self.pending_sizes, key);
        let released_presentation = self
            .pending_presentation_request
            .is_some_and(|(pending_key, _)| pending_key == key);
        let pending_cleared = self.clear_pending_workspace_selection_for(key);
        IncomingEffect {
            rebuild: self.active_connection == key
                && (pending_cleared || active_projection_cleared),
            rebuild_active: released_presentation,
            notify: true,
        }
    }
}
