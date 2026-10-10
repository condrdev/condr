use super::*;

pub(super) struct ConnectionResult {
    pub(super) key: ConnectionKey,
    pub(super) generation: u64,
    pub(super) endpoint: Endpoint,
    pub(super) result: Result<ClientConnection, String>,
    /// Set when the Server read this Client's `Hello` and closed the connection.
    pub(super) refusal: Option<condr_core::protocol::Refusal>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ConnectionStatus {
    Connecting,
    Connected,
    Disconnected,
}

pub(super) struct ServerConnection {
    pub(super) key: ConnectionKey,
    pub(super) label: String,
    pub(super) endpoint: Endpoint,
    pub(super) status: ConnectionStatus,
    /// The Server's build identity from its last `Welcome` (ADR 0027).
    pub(super) server_build: Option<String>,
    /// The person closed the "different build" mark for the current `server_build`.
    pub(super) build_notice_dismissed: bool,
    /// This window already asked whether to restart a stale local Server; asked once.
    pub(super) update_restart_offered: bool,
    /// The Session, terminals and Agents this connection holds (ADR 0039).
    pub(super) model: SessionModel,
    /// The Server process that said `ServerStopping`. It keeps accepting until it exits,
    /// so a reconnect can reach it again; its Bootstrap is refused.
    pub(super) stopping_runtime: Option<RuntimeEpoch>,
    /// This Client's own view of the Session (ADR 0021): the Workspace it shows and, per
    /// Workspace, the Tab. Never sent to the Server. A choice that no longer exists falls
    /// back to the first Workspace or Tab; `reconcile_view` moves it to a neighbour first.
    pub(super) view_workspace: Option<WorkspaceId>,
    pub(super) view_tabs: HashMap<WorkspaceId, TabId>,
    /// Panes with a clipboard image still on its way to the Server; the header says so.
    pub(super) pasting_images: HashSet<PaneId>,
    pub(super) resources: WorkspaceResources,
    /// The state of each agent's status hooks on the Server's machine, as last
    /// reported; requested when Settings shows this Server and after every action.
    pub(super) hooks: Vec<HooksReport>,
    /// Why the last hooks request failed, until the next report.
    pub(super) hooks_error: Option<String>,
    pub(super) listen: Option<String>,
    /// `[server.p2p] enabled` on the Server, as its last `Status` or `P2pSaved` said.
    pub(super) p2p: bool,
    /// What the Server process actually bound, from `Status`; `listen` and `p2p` are
    /// saved values that only a restart applies.
    pub(super) running_listen: Option<String>,
    pub(super) running_p2p: bool,
    /// The `[network.proxy]` Peer-to-peer started with, from `Status`; `None` until one
    /// arrives, so a saved proxy is not taken for a pending one.
    pub(super) running_proxy: Option<ProxySetting>,
    /// The Server's last `Status` report; `None` until the first one arrives.
    pub(super) health: Option<ServerHealth>,
    pub(super) clients: Vec<ServerClientInfo>,
    pub(super) connected_devices: Vec<String>,
    pub(super) admin_error: Option<String>,
    /// The last invite this Server issued, for the Clients page to show and copy.
    pub(super) invite: Option<ServerInvite>,
    pub(super) io: Option<ClientIo>,
    pub(super) cancellation: ConnectionCancellation,
    pub(super) connect_generation: u64,
    pub(super) subscribed: bool,
    pub(super) subscription_pending: bool,
    pub(super) bootstrap_resync_session_id: Option<SessionId>,
    /// Set by a subscription rejection: the next Bootstrap must drop pending layout
    /// projections, because responses may have been lost to writer lag.
    pub(super) recover_after_bootstrap: bool,
    /// When a message from a newer protocol last forced a Bootstrap. A second within
    /// [`UNKNOWN_MESSAGE_WINDOW`] disconnects rather than loop (ADR 0028).
    pub(super) unknown_message_at: Option<Instant>,
    /// Why the connection is not up: the connect attempt's or the disconnect's reason.
    /// Only that; a refused command is a toast.
    pub(super) error: Option<String>,
    /// The typed reason behind `error` when the Server refused the handshake.
    pub(super) refusal: Option<condr_core::protocol::Refusal>,
    /// Set while the GUI reconnects on its own, after a restart it asked for or a
    /// connection that ended without it; cleared on success or at the deadline.
    pub(super) reconnect_deadline: Option<Instant>,
    /// When an established connection last went down, for `RECONNECT_GRACE`.
    pub(super) disconnected_at: Option<Instant>,
    /// When the Ping sent after the machine woke left, until a Pong answers it.
    pub(super) wake_probe: Option<Instant>,
    pub(super) next_layout_request_id: u64,
}

pub(super) struct BootstrapApplication {
    pub(super) rebuild: bool,
    pub(super) resubscribe: bool,
    /// A new authority or a rejected subscription: drop pending layout projections and
    /// subscribe afresh.
    pub(super) recovery: bool,
    /// A different Server/runtime/Session: cached GUI state for the connection is stale.
    pub(super) authority_changed: bool,
}

impl Drop for ServerConnection {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

impl ServerConnection {
    /// One sentence when the Server is another build than this window, with the side to
    /// update when the versions say which is older; `None` when they match or it was
    /// dismissed.
    pub(super) fn build_notice(&self) -> Option<String> {
        if self.build_notice_dismissed {
            return None;
        }
        let server = self.server_build.as_deref()?;
        let this = condr_core::build_identity();
        let label = &self.label;
        if local_server_is_stale(&self.endpoint, this, server) {
            return Some(format!(
                "{label} still runs Condr {server}; this window is {this}. \
                 Restart Condr on {label} to switch."
            ));
        }
        Some(match condr_core::compare_builds(this, server) {
            condr_core::BuildComparison::Same => return None,
            condr_core::BuildComparison::OtherOlder => format!(
                "{label} runs Condr {server}; this window is {this}. Update Condr on {label}."
            ),
            condr_core::BuildComparison::OtherNewer => format!(
                "{label} runs Condr {server}; this window is {this}. Update Condr on this device."
            ),
            condr_core::BuildComparison::DifferentCommit => format!(
                "{label} runs a different Condr build ({server}); this window is {this}. \
                 Update both to the same version."
            ),
        })
    }

    /// Whether to ask, once, to restart this machine's Server into this window's build.
    pub(super) fn offers_update_restart(&self) -> bool {
        !self.update_restart_offered
            && self.server_build.as_deref().is_some_and(|server| {
                local_server_is_stale(&self.endpoint, condr_core::build_identity(), server)
            })
    }

    pub(super) fn new(key: ConnectionKey, label: String, endpoint: Endpoint) -> Self {
        Self {
            key,
            label,
            endpoint,
            status: ConnectionStatus::Disconnected,
            server_build: None,
            build_notice_dismissed: false,
            update_restart_offered: false,
            model: SessionModel::default(),
            stopping_runtime: None,
            view_workspace: None,
            view_tabs: HashMap::new(),
            pasting_images: HashSet::new(),
            resources: WorkspaceResources::default(),
            hooks: Vec::new(),
            hooks_error: None,
            listen: None,
            p2p: false,
            running_listen: None,
            running_p2p: false,
            running_proxy: None,
            health: None,
            clients: Vec::new(),
            connected_devices: Vec::new(),
            admin_error: None,
            invite: None,
            reconnect_deadline: None,
            io: None,
            cancellation: ConnectionCancellation::default(),
            connect_generation: 0,
            subscribed: false,
            subscription_pending: false,
            bootstrap_resync_session_id: None,
            recover_after_bootstrap: false,
            unknown_message_at: None,
            error: None,
            refusal: None,
            disconnected_at: None,
            wake_probe: None,
            next_layout_request_id: 1,
        }
    }

    pub(super) fn can_mutate(&self) -> bool {
        self.is_synchronized()
    }

    pub(super) fn reset_sync_state(&mut self) {
        self.model.attention.clear();
        self.pasting_images.clear();
        self.subscribed = false;
        self.subscription_pending = false;
        self.bootstrap_resync_session_id = None;
        self.recover_after_bootstrap = false;
        self.wake_probe = None;
    }

    pub(super) fn is_synchronized(&self) -> bool {
        self.status == ConnectionStatus::Connected
            && self.subscribed
            && self.bootstrap_resync_session_id.is_none()
    }

    pub(super) fn session(&self) -> Option<&Session> {
        self.model.session()
    }

    /// The Session the window shows: kept while connected or retrying on its own, the
    /// frozen Panes then behind a veil. A device that gave up or was disconnected shows
    /// none, so its Workspaces leave the sidebar and its body is the disconnected page
    /// (ADR 0020); `session` still holds it for the reconnect to land where it was.
    pub(super) fn presented_session(&self) -> Option<&Session> {
        (self.status == ConnectionStatus::Connected || self.reconnect_deadline.is_some())
            .then_some(self.model.session())
            .flatten()
    }

    /// Carries this Client's view across a structure the model just replaced.
    fn carry_view(&mut self, previous: Option<Session>) {
        if let Some(before) = previous
            && let Some(after) = self.model.session().cloned()
        {
            self.reconcile_view(&before, &after);
        }
    }

    /// What a Pane is called: the title its program set, else its agent's name.
    pub(super) fn pane_title(&self, pane_id: PaneId) -> Option<String> {
        self.model.pane_title(pane_id)
    }

    /// The Workspace this Client shows: its choice while that exists, else the first.
    pub(super) fn viewed_workspace_id(&self, session: &Session) -> Option<WorkspaceId> {
        self.view_workspace
            .filter(|id| session.workspace(*id).is_some())
            .or_else(|| session.workspaces().first().map(Workspace::id))
    }

    /// The Tab this Client shows in `workspace_id`: its choice while that exists, else the
    /// first.
    pub(super) fn viewed_tab_id(
        &self,
        session: &Session,
        workspace_id: WorkspaceId,
    ) -> Option<TabId> {
        let workspace = session.workspace(workspace_id)?;
        self.view_tabs
            .get(&workspace_id)
            .copied()
            .filter(|id| workspace.tab(*id).is_some())
            .or_else(|| workspace.tabs().first().map(Tab::id))
    }

    pub(super) fn viewed(&self, session: &Session) -> Option<(WorkspaceId, TabId)> {
        let workspace_id = self.viewed_workspace_id(session)?;
        Some((workspace_id, self.viewed_tab_id(session, workspace_id)?))
    }

    /// Shows `workspace_id`, and `tab_id` in it when given; an unknown id is ignored.
    /// Returns whether the view changed.
    pub(super) fn set_view(&mut self, workspace_id: WorkspaceId, tab_id: Option<TabId>) -> bool {
        let Some(session) = self.model.session() else {
            return false;
        };
        let before = self.viewed(session);
        if session.workspace(workspace_id).is_none() {
            return false;
        }
        self.view_workspace = Some(workspace_id);
        if let Some(tab_id) = tab_id
            && session
                .workspace(workspace_id)
                .is_some_and(|workspace| workspace.tab(tab_id).is_some())
        {
            self.view_tabs.insert(workspace_id, tab_id);
        }
        self.viewed(session) != before
    }

    /// The structure changed: a shown Workspace or Tab that closed gives way to the one
    /// that took its place, else the last, as browser tabs do; other choices stay.
    pub(super) fn reconcile_view(&mut self, before: &Session, after: &Session) {
        if let Some(workspace_id) = self.view_workspace
            && after.workspace(workspace_id).is_none()
        {
            self.view_workspace = before
                .workspaces()
                .iter()
                .position(|workspace| workspace.id() == workspace_id)
                .and_then(|ix| after.workspaces().get(ix).or(after.workspaces().last()))
                .map(Workspace::id);
        }
        self.view_tabs
            .retain(|workspace_id, _| after.workspace(*workspace_id).is_some());
        for (workspace_id, tab_id) in &mut self.view_tabs {
            let workspace = after.workspace(*workspace_id).expect("retained above");
            if workspace.tab(*tab_id).is_some() {
                continue;
            }
            if let Some(next) = before
                .workspace(*workspace_id)
                .and_then(|old| old.tabs().iter().position(|tab| tab.id() == *tab_id))
                .and_then(|ix| workspace.tabs().get(ix).or(workspace.tabs().last()))
            {
                *tab_id = next.id();
            }
        }
    }

    pub(super) fn apply_bootstrap(
        &mut self,
        bootstrap: SessionBootstrap,
    ) -> Result<BootstrapApplication, String> {
        let previous_layout = self.dock_projection();
        let resubscribe = self.bootstrap_resync_session_id.is_some() && !self.subscribed;
        // Validated before any identity, cursor, terminal or view state changes.
        let change = self.model.apply_bootstrap(bootstrap)?;
        let rejected_recovery = std::mem::take(&mut self.recover_after_bootstrap);
        if change.authority_changed {
            self.subscribed = false;
            self.subscription_pending = false;
        }
        self.carry_view(change.previous);
        self.resources.reset();
        self.status = ConnectionStatus::Connected;
        self.bootstrap_resync_session_id = None;
        self.error = None;
        self.disconnected_at = None;
        Ok(BootstrapApplication {
            rebuild: previous_layout != self.dock_projection(),
            resubscribe: resubscribe || change.authority_changed,
            recovery: change.authority_changed || rejected_recovery,
            authority_changed: change.authority_changed,
        })
    }

    /// Replaces structure atomically while preserving live terminals and the Client's view.
    pub(super) fn apply_layout(
        &mut self,
        snapshot: SessionSnapshot,
        zoomed_panes: Vec<PaneId>,
    ) -> Result<bool, String> {
        let previous_layout = self.dock_projection();
        let previous_viewer = self.presented_viewer();
        let previous = self.model.apply_layout(snapshot, zoomed_panes)?;
        self.carry_view(previous);
        // Retargeting a viewer also needs to refresh its Editor, even without a Dock.
        Ok(self.dock_projection() != previous_layout || self.presented_viewer() != previous_viewer)
    }

    /// The viewer Tab this Client shows, if the shown Tab is one: its identity and file.
    /// A retarget changes the file and nothing the Dock projection sees.
    pub(super) fn presented_viewer(&self) -> Option<(TabId, RelativePathBuf)> {
        let session = self.session()?;
        let (_, tab_id) = self.viewed(session)?;
        let tab = session.tab(tab_id)?;
        let path = tab
            .diff()
            .map(|diff| diff.path())
            .or_else(|| tab.file().map(|file| file.path()))?;
        Some((tab.id(), path.to_relative_path_buf()))
    }

    pub(super) fn dock_projection(&self) -> Option<PaneLayout> {
        let session = self.session()?;
        let (_, tab_id) = self.viewed(session)?;
        let tab = session.tab(tab_id)?;
        self.model
            .zoomed_panes
            .iter()
            .copied()
            .find(|pane_id| tab.panes().iter().any(|pane| pane.id() == *pane_id))
            .map(PaneLayout::Pane)
            .or_else(|| tab.layout().cloned())
    }

    /// Asks the Server to install, remove or report one agent's hooks on its machine.
    /// The reply comes back as an `AgentResult` and replaces that agent's row.
    pub(super) fn send_agent_hooks(&mut self, agent: AgentKind, action: HooksAction) {
        let (Some(server_id), Some(session_id)) = (self.model.server_id, self.model.session_id)
        else {
            return;
        };
        self.send(ClientMessage::Agent {
            server_id,
            session_id,
            command: AgentCommand::Hooks { agent, action },
        });
    }

    /// Enqueues a message without changing lifecycle state. The incoming task owns
    /// disconnect delivery, including when its writer has already closed this queue.
    pub(super) fn send(&self, message: ClientMessage) -> bool {
        self.io
            .as_ref()
            .is_some_and(|io| io.outgoing.send(message).is_ok())
    }

    pub(super) fn subscribe(&mut self) {
        if self.subscription_pending {
            return;
        }
        let Some(session_id) = self.model.session_id else {
            return;
        };
        if self.send(ClientMessage::Subscribe {
            session_id,
            after_sequence: self.model.sequence,
        }) {
            self.subscribed = false;
            self.subscription_pending = true;
        }
    }

    pub(super) fn request_snapshot(&mut self) -> bool {
        let Some(session_id) = self.model.session_id else {
            return false;
        };
        self.request_snapshot_for(session_id)
    }

    pub(super) fn request_snapshot_for(&mut self, session_id: SessionId) -> bool {
        if self.bootstrap_resync_session_id == Some(session_id) {
            return false;
        }
        if !self.send(ClientMessage::SnapshotRequest { session_id }) {
            return false;
        }
        self.bootstrap_resync_session_id = Some(session_id);
        true
    }

    pub(super) fn recover_rejected_snapshot(
        &mut self,
        server_id: ServerId,
        authoritative_session_id: SessionId,
        reason: String,
    ) -> bool {
        if self.model.server_id != Some(server_id) || self.bootstrap_resync_session_id.is_none() {
            return false;
        }
        self.bootstrap_resync_session_id = None;
        self.subscribed = false;
        self.subscription_pending = false;
        tracing::warn!(
            reason,
            "the Server rejected the Bootstrap; requesting a fresh snapshot"
        );
        self.request_snapshot_for(authoritative_session_id)
    }

    /// A message from a newer protocol (ADR 0028): recover as its stream requires and
    /// report whether to repaint, or `None` when it is the second within
    /// [`UNKNOWN_MESSAGE_WINDOW`] and the connection should end instead of looping.
    pub(super) fn receive_unknown(
        &mut self,
        unknown: UnknownMessage,
        now: Instant,
    ) -> Option<bool> {
        if self
            .unknown_message_at
            .is_some_and(|at| now.duration_since(at) < UNKNOWN_MESSAGE_WINDOW)
        {
            return None;
        }
        self.unknown_message_at = Some(now);
        Some(match unknown {
            // The reader already dropped the visual state and asked for a Bootstrap.
            UnknownMessage::TerminalFrame => false,
            UnknownMessage::Event { .. } => {
                self.subscribed = false;
                self.subscription_pending = false;
                self.request_snapshot()
            }
            // It may have been a reply something waits on: recover as for a reply lost
            // to writer lag, which also clears projections and reacquires control.
            UnknownMessage::Reply => match (self.model.server_id, self.model.session_id) {
                (Some(server_id), Some(session_id)) => {
                    self.recover_rejected_subscription(server_id, session_id)
                }
                _ => false,
            },
        })
    }

    pub(super) fn recover_rejected_subscription(
        &mut self,
        server_id: ServerId,
        authoritative_session_id: SessionId,
    ) -> bool {
        // Writer overflow may already have discarded a queued LayoutApplied,
        // so the rejection is actionable even while a visual-gap snapshot is in flight.
        if self.model.server_id != Some(server_id) {
            return false;
        }
        self.subscription_pending = false;
        self.subscribed = false;
        self.recover_after_bootstrap = true;
        self.request_snapshot_for(authoritative_session_id);
        true
    }
}

/// One invite as the Server printed it: a link per enabled transport (ADR 0026).
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ServerInvite {
    pub(super) tcp: Option<String>,
    pub(super) p2p: Option<String>,
    pub(super) expires_in_secs: u64,
}

/// Runtime figures from the Server's `Status` reply, shown on the General settings page.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ServerHealth {
    pub(super) version: String,
    pub(super) uptime_secs: u64,
    pub(super) workspaces: u32,
    pub(super) tabs: u32,
    pub(super) panes: u32,
    pub(super) agents: u32,
    pub(super) clients: u32,
    pub(super) recent_errors: Vec<ServerLogRecord>,
}

/// This machine's Server runs an older or a different build than this window: what an
/// update leaves behind, since installing never stops a running Server (ADR 0016). A
/// restart starts the `condr` that came with this window. A newer Server is left alone,
/// because restarting it would downgrade.
fn local_server_is_stale(endpoint: &Endpoint, this: &str, server: &str) -> bool {
    matches!(endpoint, Endpoint::Local(_))
        && matches!(
            condr_core::compare_builds(this, server),
            condr_core::BuildComparison::OtherOlder | condr_core::BuildComparison::DifferentCommit
        )
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the app module's glob brings in GPUI's `#[test]` macro.
    use super::local_server_is_stale;
    use condr_server::{Endpoint, SshEndpoint};

    #[test]
    fn only_an_older_or_other_local_server_is_stale() {
        let local = Endpoint::local("/tmp/condr.sock");
        assert!(local_server_is_stale(&local, "0.2.0", "0.1.9"));
        assert!(local_server_is_stale(&local, "0.2.0+b", "0.2.0+a"));
        assert!(!local_server_is_stale(&local, "0.2.0", "0.2.0"));
        assert!(!local_server_is_stale(&local, "0.2.0", "0.3.0"));
        let ssh = Endpoint::Ssh(SshEndpoint::parse("ssh://host").unwrap());
        assert!(!local_server_is_stale(&ssh, "0.2.0", "0.1.9"));
    }
}
