use super::*;

pub(super) struct ConnectionResult {
    pub(super) key: ConnectionKey,
    pub(super) generation: u64,
    pub(super) endpoint: Endpoint,
    pub(super) result: Result<ClientConnection, String>,
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
    pub(super) server_id: Option<ServerId>,
    pub(super) runtime_epoch: Option<RuntimeEpoch>,
    pub(super) session_id: Option<SessionId>,
    pub(super) sequence: u64,
    pub(super) snapshot: SessionSnapshot,
    pub(super) terminals: HashMap<PaneId, ClientTerminal>,
    pub(super) terminal_titles: HashMap<PaneId, String>,
    pub(super) terminal_hyperlinks: HashMap<PaneId, TerminalHyperlinkBudget>,
    pub(super) agents: HashMap<PaneId, AgentSnapshot>,
    pub(super) agent_trackers: HashMap<PaneId, AgentTracker>,
    /// Panes that rang BEL while not focused; cleared when the Terminal gains focus.
    pub(super) attention: HashSet<PaneId>,
    /// Panes with a clipboard image still on its way to the Server; the header says so.
    pub(super) pasting_images: HashSet<PaneId>,
    pub(super) workspace_git: HashMap<WorkspaceId, WorkspaceGitSnapshot>,
    /// File diffs the Server answered, by Workspace and path (ADR 0017). Cleared for a
    /// Workspace whenever its Git state changes, so a shown diff is refetched.
    pub(super) diffs: HashMap<(WorkspaceId, RelativePathBuf), Result<FileDiff, String>>,
    /// Bumped whenever `diffs` changes, so a Diff Tab knows its text is stale.
    pub(super) diffs_generation: u64,
    /// Directory listings the Server answered for the Files sidebar (ADR 0018), by
    /// Workspace and root-relative path; a Git change asks for every cached one again.
    pub(super) directories:
        HashMap<(WorkspaceId, RelativePathBuf), Result<DirectoryListing, String>>,
    /// File contents the Server answered for Preview Tabs, refreshed the same way, each
    /// with the `files_generation` it landed at. A refetch that returns the same content
    /// keeps its generation, so the Preview Tab's Editor is left alone (no re-highlight,
    /// no scroll reset).
    pub(super) files: HashMap<(WorkspaceId, RelativePathBuf), (u64, Result<FileContent, String>)>,
    /// Bumped whenever an entry of `files` changes.
    pub(super) files_generation: u64,
    pub(super) zoomed_panes: HashSet<PaneId>,
    /// Server-owned preferences from the Bootstrap, kept current by events.
    pub(super) settings: ServerSettings,
    /// The state of each agent's status hooks on the Server's machine, as last
    /// reported; requested when Settings shows this Server and after every action.
    pub(super) hooks: Vec<HooksReport>,
    /// Why the last hooks request failed, until the next report.
    pub(super) hooks_error: Option<String>,
    pub(super) listen: Option<String>,
    /// The Server's last `Status` report; `None` until the first one arrives.
    pub(super) health: Option<ServerHealth>,
    pub(super) clients: Vec<ServerClientInfo>,
    pub(super) connected_devices: Vec<String>,
    pub(super) admin_error: Option<String>,
    /// The last invite this Server issued and how long it said it was valid, for the
    /// Clients page to show and copy.
    pub(super) invite: Option<(String, u64)>,
    pub(super) io: Option<ClientIo>,
    pub(super) cancellation: ConnectionCancellation,
    pub(super) connect_generation: u64,
    pub(super) controlling: bool,
    pub(super) subscribed: bool,
    pub(super) subscription_pending: bool,
    pub(super) control_retry_attempts: u8,
    pub(super) control_retry_scheduled: bool,
    pub(super) bootstrap_resync_session_id: Option<SessionId>,
    /// Set by a subscription rejection: the next Bootstrap must re-acquire control and drop
    /// pending layout projections, because responses may have been lost to writer lag.
    pub(super) reacquire_after_bootstrap: bool,
    /// Why the connection is not up: the connect attempt's or the disconnect's reason.
    /// Only that; a refused command is a toast and a denied control is `control_denied`.
    pub(super) error: Option<String>,
    /// The Server's reason for not granting this Client control, while it stands. Not an
    /// error: the Session is still viewable, and the busy case retries on its own.
    pub(super) control_denied: Option<String>,
    /// Set while the GUI reconnects on its own, after a restart it asked for or a
    /// connection that ended without it; cleared on success or at the deadline.
    pub(super) reconnect_deadline: Option<Instant>,
    /// When an established connection last went down, for `RECONNECT_GRACE`.
    pub(super) disconnected_at: Option<Instant>,
    pub(super) next_layout_request_id: u64,
}

pub(super) struct BootstrapApplication {
    pub(super) rebuild: bool,
    pub(super) resubscribe: bool,
    /// Re-send AcquireControl and drop pending layout projections.
    pub(super) reacquire_control: bool,
    /// A different Server/runtime/Session: cached GUI state for the connection is stale.
    pub(super) authority_changed: bool,
}

impl Drop for ServerConnection {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

impl ServerConnection {
    pub(super) fn new(key: ConnectionKey, label: String, endpoint: Endpoint) -> Self {
        Self {
            key,
            label,
            endpoint,
            status: ConnectionStatus::Disconnected,
            server_id: None,
            runtime_epoch: None,
            session_id: None,
            sequence: 0,
            snapshot: Session::new().snapshot(),
            terminals: HashMap::new(),
            terminal_titles: HashMap::new(),
            terminal_hyperlinks: HashMap::new(),
            agents: HashMap::new(),
            agent_trackers: HashMap::new(),
            attention: HashSet::new(),
            pasting_images: HashSet::new(),
            workspace_git: HashMap::new(),
            diffs: HashMap::new(),
            diffs_generation: 0,
            directories: HashMap::new(),
            files: HashMap::new(),
            files_generation: 0,
            zoomed_panes: HashSet::new(),
            settings: ServerSettings::default(),
            hooks: Vec::new(),
            hooks_error: None,
            listen: None,
            health: None,
            clients: Vec::new(),
            connected_devices: Vec::new(),
            admin_error: None,
            invite: None,
            reconnect_deadline: None,
            io: None,
            cancellation: ConnectionCancellation::default(),
            connect_generation: 0,
            controlling: false,
            subscribed: false,
            subscription_pending: false,
            control_retry_attempts: 0,
            control_retry_scheduled: false,
            bootstrap_resync_session_id: None,
            reacquire_after_bootstrap: false,
            error: None,
            control_denied: None,
            disconnected_at: None,
            next_layout_request_id: 1,
        }
    }

    pub(super) fn can_mutate(&self) -> bool {
        self.is_synchronized() && self.controlling
    }

    pub(super) fn reset_sync_state(&mut self) {
        self.controlling = false;
        self.control_denied = None;
        self.attention.clear();
        self.pasting_images.clear();
        self.subscribed = false;
        self.subscription_pending = false;
        self.control_retry_attempts = 0;
        self.control_retry_scheduled = false;
        self.bootstrap_resync_session_id = None;
        self.reacquire_after_bootstrap = false;
    }

    pub(super) fn is_synchronized(&self) -> bool {
        self.status == ConnectionStatus::Connected
            && self.subscribed
            && self.bootstrap_resync_session_id.is_none()
    }

    pub(super) fn apply_bootstrap(&mut self, bootstrap: SessionBootstrap) -> BootstrapApplication {
        let previous_layout = self.dock_projection();
        let authority_changed = self.server_id != Some(bootstrap.server_id)
            || self.runtime_epoch != Some(bootstrap.runtime_epoch)
            || self.session_id != Some(bootstrap.session_id);
        let resubscribe = self.bootstrap_resync_session_id.is_some() && !self.subscribed;
        let rejected_recovery = std::mem::take(&mut self.reacquire_after_bootstrap);
        if authority_changed {
            self.controlling = false;
            self.subscribed = false;
            self.subscription_pending = false;
            self.control_retry_attempts = 0;
            self.control_retry_scheduled = false;
            self.agent_trackers.clear();
            self.attention.clear();
        }
        self.server_id = Some(bootstrap.server_id);
        self.runtime_epoch = Some(bootstrap.runtime_epoch);
        self.session_id = Some(bootstrap.session_id);
        self.sequence = bootstrap.sequence;
        self.snapshot = bootstrap.snapshot;
        // Server-authoritative; the Bootstrap usually lands before ControlGranted, so keep it
        // regardless of `controlling` and let presentation gate on control instead.
        self.attention = bootstrap
            .terminals
            .iter()
            .filter_map(|terminal| terminal.attention.then_some(terminal.pane_id))
            .collect();
        self.terminals.clear();
        self.terminal_titles.clear();
        self.terminal_hyperlinks.clear();
        for mut terminal in bootstrap.terminals {
            let pane_id = terminal.pane_id;
            if let Some(title) = terminal.title.take() {
                self.terminal_titles.insert(pane_id, title);
            }
            self.terminal_hyperlinks
                .insert(pane_id, TerminalHyperlinkBudget::new(&mut terminal.view));
            self.terminals.insert(pane_id, terminal.into());
        }
        self.agents = bootstrap
            .agents
            .into_iter()
            .map(|agent| (agent.pane_id, agent.agent))
            .collect();
        self.agent_trackers
            .retain(|pane_id, _| self.agents.contains_key(pane_id));
        for (&pane_id, agent) in &self.agents {
            self.agent_trackers
                .entry(pane_id)
                .and_modify(|tracker| tracker.update(agent.state, false))
                .or_insert_with(|| AgentTracker::new(agent.state));
        }
        self.diffs.clear();
        self.diffs_generation += 1;
        self.directories.clear();
        self.files.clear();
        self.files_generation += 1;
        self.workspace_git = bootstrap
            .workspace_git
            .into_iter()
            .map(|git| (git.workspace_id, git))
            .collect();
        self.zoomed_panes = bootstrap.zoomed_panes.into_iter().collect();
        self.settings = bootstrap.settings;
        self.status = ConnectionStatus::Connected;
        self.bootstrap_resync_session_id = None;
        self.error = None;
        self.disconnected_at = None;
        BootstrapApplication {
            rebuild: previous_layout != self.dock_projection(),
            resubscribe: resubscribe || authority_changed,
            reacquire_control: authority_changed || rejected_recovery,
            authority_changed,
        }
    }

    /// The viewer Tab the Session presents, if the active Tab is one: its identity and
    /// file. A retarget changes the file and nothing the Dock projection sees.
    pub(super) fn presented_viewer(&self) -> Option<(TabId, RelativePathBuf)> {
        let session = Session::restore(self.snapshot.clone()).ok()?;
        let tab = session.active_workspace()?.active_tab();
        let path = tab
            .diff()
            .map(|diff| diff.path())
            .or_else(|| tab.file().map(|file| file.path()))?;
        Some((tab.id(), path.to_relative_path_buf()))
    }

    pub(super) fn dock_projection(&self) -> Option<PaneLayout> {
        let session = Session::restore(self.snapshot.clone()).ok()?;
        let tab = session.active_workspace()?.active_tab();
        self.zoomed_panes
            .iter()
            .copied()
            .find(|pane_id| tab.panes().iter().any(|pane| pane.id() == *pane_id))
            .map(PaneLayout::Pane)
            .or_else(|| tab.layout().cloned())
    }

    /// Asks the Server to install, remove or report one agent's hooks on its machine.
    /// The reply comes back as an `AgentResult` and replaces that agent's row.
    pub(super) fn send_agent_hooks(&mut self, agent: AgentKind, action: HooksAction) {
        let (Some(server_id), Some(session_id)) = (self.server_id, self.session_id) else {
            return;
        };
        self.send(ClientMessage::Agent {
            server_id,
            session_id,
            command: AgentCommand::Hooks { agent, action },
        });
    }

    pub(super) fn send(&mut self, message: ClientMessage) {
        let failed = self
            .io
            .as_ref()
            .is_none_or(|io| io.outgoing.send(message).is_err());
        if failed {
            self.status = ConnectionStatus::Disconnected;
            self.controlling = false;
            self.attention.clear();
            self.subscribed = false;
            self.subscription_pending = false;
            self.bootstrap_resync_session_id = None;
            self.reacquire_after_bootstrap = false;
            self.error = Some("Disconnected from the device".into());
            self.io = None;
        }
    }

    pub(super) fn subscribe(&mut self) {
        if self.subscription_pending {
            return;
        }
        let Some(session_id) = self.session_id else {
            return;
        };
        self.subscribed = false;
        self.subscription_pending = true;
        self.send(ClientMessage::Subscribe {
            session_id,
            after_sequence: self.sequence,
        });
    }

    pub(super) fn request_snapshot(&mut self) -> bool {
        let Some(session_id) = self.session_id else {
            return false;
        };
        self.request_snapshot_for(session_id)
    }

    pub(super) fn request_snapshot_for(&mut self, session_id: SessionId) -> bool {
        if self.bootstrap_resync_session_id == Some(session_id) {
            return false;
        }
        self.bootstrap_resync_session_id = Some(session_id);
        self.send(ClientMessage::SnapshotRequest { session_id });
        true
    }

    pub(super) fn recover_rejected_snapshot(
        &mut self,
        server_id: ServerId,
        authoritative_session_id: SessionId,
        reason: String,
    ) -> bool {
        if self.server_id != Some(server_id) || self.bootstrap_resync_session_id.is_none() {
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

    pub(super) fn recover_rejected_subscription(
        &mut self,
        server_id: ServerId,
        authoritative_session_id: SessionId,
    ) -> bool {
        // Writer overflow may already have discarded a queued ControlGranted or LayoutApplied,
        // so the rejection is actionable even while a visual-gap snapshot is in flight.
        if self.server_id != Some(server_id) {
            return false;
        }
        self.subscription_pending = false;
        self.subscribed = false;
        self.reacquire_after_bootstrap = true;
        self.request_snapshot_for(authoritative_session_id);
        true
    }
}

/// Runtime figures from the Server's `Status` reply, shown on the Daemon settings page.
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
