use super::*;

impl Condr {
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
                // The Bootstrap replaced every listing, file and diff cache; answers to
                // requests sent before it were dropped with the old connection.
                self.clear_pending_requests(key);
                self.sync_sidebar_workspace_open(cx);
                if application.reacquire_control {
                    // Layout responses may have been lost to writer lag; this Bootstrap is
                    // the authoritative layout, so nothing stays pending against it.
                    self.clear_pending_workspace_selection_for(key);
                    _ = self.clear_pending_projections_for(key);
                }

                let bootstrap_sequence = self.connections[index].sequence;
                let active_projection_resolved =
                    self.resolve_projections_at(key, bootstrap_sequence);
                let (pending_workspace_ready, pending_workspace_failed) =
                    self.resolve_pending_workspace_at(key, index, bootstrap_sequence);
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
            Incoming::ImageSent(pane_id) => {
                return IncomingEffect {
                    notify: self.connections[index].pasting_images.remove(&pane_id),
                    ..IncomingEffect::default()
                };
            }
            Incoming::Disconnected(error) => {
                let effect = self.mark_disconnected(key, index, error);
                self.begin_reconnect(key, cx);
                return effect;
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
                let mut git_changed = false;
                match event {
                    SessionEvent::LayoutChanged {
                        snapshot,
                        zoomed_panes,
                    } => {
                        // The event is the new structure; apply it in place. No Bootstrap
                        // round trip, so the UI never enters the "not synchronized" state
                        // and terminal views are untouched.
                        let connection = &mut self.connections[index];
                        let previous_layout = connection.dock_projection();
                        let previous_viewer = connection.presented_viewer();
                        connection.snapshot = snapshot;
                        connection.zoomed_panes = zoomed_panes.into_iter().collect();
                        // A viewer Tab retargeted to another file needs the rebuild too:
                        // that is where its Editor learns to ask for the new content.
                        let layout_changed = connection.dock_projection() != previous_layout
                            || connection.presented_viewer() != previous_viewer;
                        // Terminals of closed Panes go; a new Pane's terminal arrives with
                        // its first full frame.
                        if let Ok(session) = Session::restore(connection.snapshot.clone()) {
                            let live: HashSet<PaneId> = session
                                .workspaces()
                                .iter()
                                .flat_map(|workspace| workspace.tabs())
                                .flat_map(|tab| tab.panes())
                                .map(|pane| pane.id())
                                .collect();
                            connection
                                .terminals
                                .retain(|pane_id, _| live.contains(pane_id));
                            connection
                                .terminal_hyperlinks
                                .retain(|pane_id, _| live.contains(pane_id));
                            connection
                                .terminal_titles
                                .retain(|pane_id, _| live.contains(pane_id));
                        }
                        self.prune_dock_cache(key);
                        self.sync_sidebar_workspace_open(cx);
                        return self.settle_layout(key, index, sequence, layout_changed);
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
                            let previous = self.connections[index]
                                .agents
                                .insert(pane_id, agent.clone());
                            // The Server only publishes changed snapshots, so Unknown here is
                            // a new process, not a repeat that would reopen a manual collapse.
                            let started = previous
                                .as_ref()
                                .is_none_or(|previous| previous.kind != agent.kind)
                                || agent.state == AgentState::Unknown;
                            if started
                                && let Some(workspace_id) =
                                    Session::restore(self.connections[index].snapshot.clone())
                                        .ok()
                                        .and_then(|session| {
                                            session
                                                .workspace_for_pane(pane_id)
                                                .map(|workspace| workspace.id())
                                        })
                            {
                                self.sidebar_workspace_open
                                    .entry((key, workspace_id))
                                    .or_insert_with(|| cx.new(|_| true))
                                    .update(cx, |open, cx| {
                                        *open = true;
                                        cx.notify();
                                    });
                            }
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
                            if !visible {
                                self.notify_agent_change(
                                    key,
                                    pane_id,
                                    previous.map(|previous| previous.state),
                                    &agent,
                                    cx,
                                );
                            }
                        } else {
                            self.connections[index].agents.remove(&pane_id);
                            self.connections[index].agent_trackers.remove(&pane_id);
                        }
                        notify = true;
                    }
                    SessionEvent::WorkspaceGitChanged { workspace_id, git } => {
                        let connection = &mut self.connections[index];
                        if let Some(git) = git {
                            connection.workspace_git.insert(workspace_id, git);
                        } else {
                            connection.workspace_git.remove(&workspace_id);
                        }
                        // The working tree moved: every diff of it is stale, and a Diff Tab
                        // showing one asks again on the rebuild.
                        connection
                            .diffs
                            .retain(|(diff_workspace, _), _| *diff_workspace != workspace_id);
                        connection.diffs_generation += 1;
                        self.pending_diffs
                            .retain(|(pending_key, pending_workspace, _)| {
                                *pending_key != key || *pending_workspace != workspace_id
                            });
                        git_changed = true;
                        notify = true;
                    }
                    SessionEvent::WorkspaceFilesChanged { workspace_id } => {
                        // Listings and previews keep showing while fresh ones are fetched.
                        self.refresh_workspace_files(key, workspace_id);
                        notify = true;
                    }
                    SessionEvent::TerminalTitleChanged { pane_id, title } => {
                        // Reliable metadata can precede a new Pane's first visual frame.
                        let titles = &mut self.connections[index].terminal_titles;
                        match title {
                            Some(title) => {
                                titles.insert(pane_id, title);
                            }
                            None => {
                                titles.remove(&pane_id);
                            }
                        }
                        notify = true;
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
                    // A presented Diff Tab refetches its file on the rebuild.
                    rebuild: git_changed && self.active_connection == key,
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
                ..
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
                // The origin client gets the LayoutChanged event before this reply, so the
                // structure is usually already in; settle against it now rather than waiting
                // for a Bootstrap that no longer comes.
                let applied = self.connections[index].sequence;
                if applied >= sequence {
                    return self.settle_layout(key, index, applied, false);
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
                // A refused or failed admin command; a pending restart is not happening.
                self.connections[index].error = Some(message);
                self.connections[index].reconnect_deadline = None;
                IncomingEffect {
                    notify: true,
                    ..IncomingEffect::default()
                }
            }
            ServerMessage::ServerAdmin(response) => {
                let connection = &mut self.connections[index];
                connection.admin_error = None;
                match response {
                    ServerAdminResponse::Status {
                        listen,
                        connected,
                        version,
                        uptime_secs,
                        workspaces,
                        tabs,
                        panes,
                        agents,
                        clients,
                        recent_errors,
                    } => {
                        connection.listen = listen;
                        connection.connected_devices = connected;
                        connection.health = Some(ServerHealth {
                            version,
                            uptime_secs,
                            workspaces,
                            tabs,
                            panes,
                            agents,
                            clients,
                            recent_errors,
                        });
                    }
                    ServerAdminResponse::Clients { clients, connected } => {
                        connection.clients = clients;
                        connection.connected_devices = connected;
                    }
                    ServerAdminResponse::Invite {
                        address,
                        expires_in_secs,
                    } => {
                        cx.write_to_clipboard(ClipboardItem::new_string(address.clone()));
                        connection.invite = Some((address, expires_in_secs));
                    }
                    // The stored listen address changed: a pending invite still points at
                    // the old port, and none can be issued without a listener.
                    ServerAdminResponse::ListenSaved { listen } => {
                        connection.listen = listen;
                        connection.invite = None;
                    }
                    ServerAdminResponse::Revoked { .. } => {
                        connection.send(ClientMessage::ServerAdmin {
                            server_id: connection.server_id.unwrap_or(ServerId(0)),
                            command: ServerAdminCommand::Clients,
                        });
                    }
                }
                if let Some(settings) = self.settings_view.as_ref().and_then(WeakEntity::upgrade) {
                    settings.update(cx, |_, cx| cx.notify());
                }
                IncomingEffect {
                    notify: true,
                    ..IncomingEffect::default()
                }
            }
            ServerMessage::GitDiff {
                workspace_id,
                path,
                result,
                ..
            } => {
                self.pending_diffs
                    .remove(&(key, workspace_id, path.clone()));
                let connection = &mut self.connections[index];
                connection.diffs.insert((workspace_id, path), result);
                connection.diffs_generation += 1;
                // The rebuild hands the answer to the Diff Tab's Editor.
                IncomingEffect {
                    rebuild: self.active_connection == key,
                    notify: true,
                    ..IncomingEffect::default()
                }
            }
            ServerMessage::Directory {
                workspace_id,
                path,
                result,
                ..
            } => {
                self.pending_directories
                    .remove(&(key, workspace_id, path.clone()));
                self.connections[index]
                    .directories
                    .insert((workspace_id, path), result);
                IncomingEffect {
                    notify: true,
                    ..IncomingEffect::default()
                }
            }
            ServerMessage::FileContent {
                workspace_id,
                path,
                result,
                ..
            } => {
                self.pending_files
                    .remove(&(key, workspace_id, path.clone()));
                let connection = &mut self.connections[index];
                let slot = (workspace_id, path);
                if connection
                    .files
                    .get(&slot)
                    .is_none_or(|(_, cached)| *cached != result)
                {
                    connection.files_generation += 1;
                    connection
                        .files
                        .insert(slot, (connection.files_generation, result));
                }
                // The rebuild hands the answer to the Preview Tab's Editor.
                IncomingEffect {
                    rebuild: self.active_connection == key,
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
            // The GUI's only agent requests are hooks, so every result is one of theirs.
            ServerMessage::AgentResult { result } => {
                let connection = &mut self.connections[index];
                match result {
                    Ok(AgentResponse::Hooks(report)) => {
                        connection.hooks_error = None;
                        match connection
                            .hooks
                            .iter_mut()
                            .find(|known| known.agent == report.agent)
                        {
                            Some(known) => *known = report,
                            None => connection.hooks.push(report),
                        }
                    }
                    Ok(_) => return IncomingEffect::default(),
                    Err(error) => connection.hooks_error = Some(error.message),
                }
                // Settings is its own window; notifying this entity repaints only the main one.
                if let Some(settings) = self.settings_view.as_ref().and_then(WeakEntity::upgrade) {
                    settings.update(cx, |_, cx| cx.notify());
                }
                IncomingEffect {
                    notify: true,
                    ..IncomingEffect::default()
                }
            }
            // Only the CLI asks for Pane text or administers devices.
            ServerMessage::Overview(_)
            | ServerMessage::OverviewTerminals { .. }
            | ServerMessage::PaneText { .. }
            | ServerMessage::DevicesRevoked { .. }
            | ServerMessage::ConnectedDevices { .. } => IncomingEffect::default(),
            ServerMessage::ServerStopping => {
                let effect = self.mark_disconnected(key, index, "Condr stopped".into());
                self.begin_reconnect(key, cx);
                effect
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
}
