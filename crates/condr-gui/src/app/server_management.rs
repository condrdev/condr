mod dialogs;

use super::*;

impl Condr {
    pub(in crate::app) fn server_admin(&mut self, key: ConnectionKey, command: ServerAdminCommand) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let Some(server_id) = connection.server_id else {
            return;
        };
        connection.send(ClientMessage::ServerAdmin { server_id, command });
    }

    pub(in crate::app) fn request_server_admin(&mut self, key: ConnectionKey) {
        self.server_admin(key, ServerAdminCommand::Status);
        self.server_admin(key, ServerAdminCommand::Clients);
    }

    pub(super) fn install_connection(
        connection: &mut ServerConnection,
        result: Result<ClientConnection, String>,
        window: &Window,
        cx: &Context<Self>,
    ) -> Result<BootstrapApplication, String> {
        let client = result?;
        let bootstrap = client.bootstrap().unwrap().clone();
        connection.cancellation = client.cancellation();
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
        connection.cancellation.cancel();
        if let Some(io) = connection.io.take() {
            let _ = io.outgoing.send(ClientMessage::Detach);
        }
        connection.connect_generation = connection.connect_generation.wrapping_add(1);
        let generation = connection.connect_generation;
        connection.status = ConnectionStatus::Connecting;
        connection.reset_sync_state();
        connection.error = None;
        let endpoint = connection.endpoint.clone();
        let cancellation = ConnectionCancellation::default();
        connection.cancellation = cancellation.clone();
        let sender = self.connect_results_tx.clone();
        clear_pending_sizes_for_bootstrap(&mut self.pending_sizes, key);
        thread::spawn(move || {
            let (connected_endpoint, result) = connect_to_server(endpoint, cancellation);
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
        connection.cancellation.cancel();
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
        if !self.server_list_writable(cx) {
            return;
        }
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
        self.save_servers(cx);
        self.clear_connection_gui_state(key);
        self.sync_sidebar_workspace_open(cx);
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
        let mut paired = false;
        let application = if let Some(connection) = self.connection_mut(key) {
            connection.endpoint = endpoint;
            match Self::install_connection(connection, result, window, cx) {
                Ok(application) => {
                    // The Server accepted the invite and recorded this device; from now on
                    // the device key alone is the credential.
                    if let Endpoint::Tcp(tcp) = &mut connection.endpoint
                        && tcp.invite.take().is_some()
                    {
                        paired = true;
                    }
                    Some(application)
                }
                Err(error) => {
                    connection.status = ConnectionStatus::Disconnected;
                    connection.error = Some(error);
                    None
                }
            }
        } else {
            None
        };
        if paired {
            self.save_servers(cx);
        }
        if let Some(application) = application {
            let presentation_before = self.pending_presentation_request;
            _ = self.clear_pending_projections_for(key);
            clear_pending_sizes_for_bootstrap(&mut self.pending_sizes, key);
            if application.authority_changed {
                self.clear_connection_gui_state(key);
            } else {
                self.prune_dock_cache(key);
            }
            self.sync_sidebar_workspace_open(cx);
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

    pub(super) fn mark_disconnected(
        &mut self,
        key: ConnectionKey,
        index: usize,
        message: String,
    ) -> IncomingEffect {
        let connection = &mut self.connections[index];
        connection.status = ConnectionStatus::Disconnected;
        connection.reset_sync_state();
        connection.error = Some(message);
        connection.cancellation.cancel();
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
