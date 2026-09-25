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

    /// Asks the Server to restart and reconnects once it has stopped. The Server replies
    /// with `ServerStopping` or an `Error`; either way the Daemon page shows the outcome.
    pub(in crate::app) fn restart_server(&mut self, key: ConnectionKey, cx: &mut Context<Self>) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        connection.reconnect_deadline = Some(Instant::now() + RESTART_RECONNECT_TIMEOUT);
        self.server_admin(key, ServerAdminCommand::Restart);
        cx.notify();
    }

    pub(super) fn clear_restart(&mut self, key: ConnectionKey) {
        if let Some(connection) = self.connection_mut(key) {
            connection.reconnect_deadline = None;
        }
    }

    /// The connection ended without this Client asking: the Server restarted, crashed or
    /// the link dropped. Keep trying for a while before leaving the error to the user.
    pub(super) fn begin_reconnect(&mut self, key: ConnectionKey, cx: &mut Context<Self>) {
        if let Some(connection) = self.connection_mut(key)
            && connection.reconnect_deadline.is_none()
        {
            connection.reconnect_deadline = Some(Instant::now() + RESTART_RECONNECT_TIMEOUT);
        }
        self.schedule_reconnect(key, cx);
        // The banner waits out the grace; nothing else would repaint when it ends.
        cx.spawn(async move |owner, cx| {
            cx.background_executor().timer(RECONNECT_GRACE).await;
            let _ = owner.update(cx, |_, cx| cx.notify());
        })
        .detach();
    }

    /// The user asked for this device: connect now and show it.
    pub(super) fn connect_server(
        &mut self,
        key: ConnectionKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.start_connect(key) {
            self.refresh_target_pane(self.active_connection);
            self.rebuild_dock(window, cx);
        }
        cx.notify();
    }

    /// While a reconnect deadline stands, retry until the device is back or the deadline
    /// passes, then leave the last error showing.
    pub(super) fn schedule_reconnect(&mut self, key: ConnectionKey, cx: &mut Context<Self>) {
        let Some(deadline) = self
            .connection(key)
            .and_then(|connection| connection.reconnect_deadline)
        else {
            return;
        };
        if Instant::now() >= deadline {
            self.clear_restart(key);
            return;
        }
        let window = self.window_handle;
        cx.spawn(async move |owner, cx| {
            cx.background_executor()
                .timer(RESTART_RECONNECT_DELAY)
                .await;
            let _ = cx.update_window(window, |_, window, cx| {
                let _ = owner.update(cx, |this, cx| {
                    let reconnect = this.connection(key).is_some_and(|connection| {
                        connection.reconnect_deadline.is_some()
                            && connection.status == ConnectionStatus::Disconnected
                    });
                    if reconnect && this.start_connect(key) {
                        this.refresh_target_pane(key);
                        this.rebuild_dock(window, cx);
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    /// After the machine sleeps, a connection whose device went away still looks up
    /// until TCP keepalive or SSH gives up, up to a minute on. A Ping each connection
    /// leaves unanswered for `WAKE_PROBE_TIMEOUT` ends it now, and the reconnect follows.
    pub(super) fn probe_connections(&mut self, cx: &mut Context<Self>) {
        let sent_at = Instant::now();
        for connection in &mut self.connections {
            if connection.status != ConnectionStatus::Connected {
                continue;
            }
            let Some(server_id) = connection.server_id else {
                continue;
            };
            connection.wake_probe = Some(sent_at);
            connection.send(ClientMessage::Ping {
                server_id,
                nonce: 0,
            });
        }
        let window = self.window_handle;
        cx.spawn(async move |owner, cx| {
            cx.background_executor().timer(WAKE_PROBE_TIMEOUT).await;
            let _ = cx.update_window(window, |_, window, cx| {
                let _ = owner.update(cx, |this, cx| {
                    let mut rebuild = false;
                    for index in 0..this.connections.len() {
                        if this.connections[index].wake_probe != Some(sent_at) {
                            continue;
                        }
                        let key = this.connections[index].key;
                        let effect = this.mark_disconnected(
                            key,
                            index,
                            "the device stopped answering".into(),
                        );
                        rebuild |= effect.rebuild;
                        this.begin_reconnect(key, cx);
                    }
                    if rebuild {
                        this.rebuild_dock(window, cx);
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    pub(super) fn install_connection(
        connection: &mut ServerConnection,
        result: Result<ClientConnection, String>,
        window: &Window,
        cx: &Context<Self>,
    ) -> Result<BootstrapApplication, String> {
        let client = result?;
        let bootstrap = client.bootstrap().unwrap().clone();
        if connection.server_build.as_deref() != Some(client.server_build()) {
            connection.server_build = Some(client.server_build().to_owned());
            connection.build_notice_dismissed = false;
        }
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
        self.active_connection()?.session()
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
        let needs_active_rebuild = self.clear_pending_projections_for(key);
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
            let refusal = std::cell::Cell::new(None);
            let (connected_endpoint, result) = connect_to_server(endpoint, cancellation, &refusal);
            let _ = sender.send_blocking(ConnectionResult {
                refusal: refusal.into_inner(),
                key,
                generation,
                endpoint: connected_endpoint,
                result,
            });
        });
        needs_active_rebuild
    }

    pub(super) fn disconnect_server(&mut self, key: ConnectionKey) -> bool {
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
        // The user chose this: no timer is running to reach the deadline, so an old
        // restart or reconnect must not keep reporting itself.
        connection.reconnect_deadline = None;
        clear_pending_sizes_for_bootstrap(&mut self.pending_sizes, key);
        active_projection_cleared
    }

    pub(super) fn remove_server(
        &mut self,
        key: ConnectionKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(error) = self.servers_error.clone() {
            self.report_error(error, cx);
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
            refusal,
        } = result;
        if !self.connection(key).is_some_and(|connection| {
            connection.status == ConnectionStatus::Connecting
                && connection.connect_generation == generation
        }) {
            return;
        }
        let mut paired = false;
        let first_bootstrap = self
            .connection(key)
            .is_some_and(|connection| connection.server_id.is_none());
        let application = if let Some(connection) = self.connection_mut(key) {
            connection.endpoint = endpoint;
            connection.refusal = refusal;
            match Self::install_connection(connection, result, window, cx) {
                Ok(application) => {
                    // The Server accepted the invite and recorded this device; from now on
                    // the device key alone is the credential.
                    let invite = match &mut connection.endpoint {
                        Endpoint::Tcp(tcp) => tcp.invite.take(),
                        Endpoint::P2p(p2p) => p2p.invite.take(),
                        Endpoint::Local(_) | Endpoint::Ssh(_) => None,
                    };
                    if invite.is_some() {
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
        if application.is_some() {
            self.clear_restart(key);
        } else {
            self.schedule_reconnect(key, cx);
        }
        if let Some(application) = application {
            _ = self.clear_pending_projections_for(key);
            clear_pending_sizes_for_bootstrap(&mut self.pending_sizes, key);
            if application.authority_changed {
                self.clear_connection_gui_state(key);
            } else {
                self.prune_dock_cache(key);
            }
            self.sync_sidebar_workspace_open(cx);
            if first_bootstrap {
                // The usual way a Server first bootstraps: the connect task, not the
                // event loop. The state file's memory of it applies here (ADR 0023).
                self.restore_server_state(key, cx);
            }
            self.acquire_and_subscribe(key);
            self.refresh_target_pane(key);
            if key == self.active_connection {
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
        connection.disconnected_at = Some(Instant::now());
        connection.cancellation.cancel();
        connection.io = None;
        let active_projection_cleared = self.clear_pending_projections_for(key);
        clear_pending_sizes_for_bootstrap(&mut self.pending_sizes, key);
        IncomingEffect {
            rebuild: self.active_connection == key && active_projection_cleared,
            rebuild_active: false,
            notify: true,
        }
    }
}
