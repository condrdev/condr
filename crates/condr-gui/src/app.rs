use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::SocketAddr;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use condr_core::protocol::{
    BootstrapAssembler, BootstrapHeader, ClientMessage, LayoutCommand, MAX_CHUNK_PAYLOAD_SIZE,
    MAX_CHUNKED_RECORD_SIZE, PaneTerminalFrame, PaneTerminalSnapshot, RuntimeEpoch, ServerId,
    ServerMessage, SessionBootstrap, SessionEvent, SessionId, TerminalFrameBatch,
    TerminalFrameChunk, WorkspaceGitSnapshot, decode_pane_terminal_frame,
};
use condr_core::{
    AgentDisplayState, AgentSnapshot, AgentTracker, PaneDirection, PaneId, PaneLayout, Session,
    SessionSnapshot, SplitDirection, TabId, TerminalCellRun, TerminalCommand, TerminalKey,
    TerminalModifiers, TerminalPosition, TerminalScroll, TerminalSelection, TerminalSize,
    TerminalViewDelta, TerminalViewFrame, WorkspaceId,
};
use condr_server::{ClientConnection, Endpoint, ServerConfig};
use gpui::prelude::FluentBuilder;
use gpui::*;
use gpui_component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_component::dialog::{Cancel, Confirm, DialogButtonProps, DialogFooter};
use gpui_component::dock::{
    BasePanel, DockArea, DockAreaRenderer, DockEvent, DockLayout, PanelEvent, PanelInfo,
    PanelState, TabGroupRenderer, TilesRenderer,
};
use gpui_component::input::{Input, InputState};
use gpui_component::menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem};
use gpui_component::resizable::{h_resizable, resizable_panel};
use gpui_component::sidebar::{
    Sidebar, SidebarCollapsible, SidebarFooter, SidebarHeader, SidebarItem,
};
use gpui_component::tooltip::Tooltip;
use gpui_component::{
    ActiveTheme as _, Collapsible, Disableable as _, ElementExt as _, Icon, IconName, IconNamed,
    Root, Selectable as _, Sizable as _, StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_component_assets::Assets;

use crate::terminal_element::{TerminalElement, TerminalElementProps, TerminalRenderCache};

mod actions;
mod config;
mod connection;
mod dialogs;
mod dock;
mod events;
mod navigation;
mod sidebar;
mod startup;
mod terminal_input;
mod workspace;

use connection::*;
use dock::*;
#[cfg(test)]
use sidebar::*;
#[cfg(test)]
use startup::connect_to_server_with;
pub(crate) use startup::run;
use startup::{connect_to_server, fixed_shortcut};
#[cfg(test)]
use terminal_input::{TerminalClipboardShortcut, terminal_clipboard_shortcut};

actions!(
    condr,
    [
        AddServer,
        ReconnectServer,
        NewWorkspace,
        NewTab,
        RenameWorkspace,
        RenameTab,
        MoveWorkspaceUp,
        MoveWorkspaceDown,
        MoveTabLeft,
        MoveTabRight,
        ClosePane,
        CloseTab,
        CloseWorkspace,
        NextTab,
        PreviousTab,
        SplitRight,
        SplitDown,
        FocusLeft,
        FocusRight,
        FocusUp,
        FocusDown,
        ResizeLeft,
        ResizeRight,
        ResizeUp,
        ResizeDown,
        SwapLeft,
        SwapRight,
        SwapUp,
        SwapDown,
        ToggleZoom
    ]
);

pub(crate) type ConnectionKey = u64;

const DEFAULT_WINDOW_SIZE: Size<Pixels> = size(px(1280.0), px(720.0));
const CONNECTION_RESULT_BUFFER_CAPACITY: usize = 16;
const SERVER_EVENT_BUFFER_CAPACITY: usize = 256;
const CONTROL_RETRY_DELAY: Duration = Duration::from_millis(50);
const MAX_CONTROL_RETRY_ATTEMPTS: u8 = 20;
const CONTROL_BUSY_REASON: &str = "another client controls this Session";
const ACTIVE_PANE_BORDER_RGB: u32 = 0x0078d4;
const INITIAL_SIDEBAR_WIDTH: Pixels = px(240.);
const WORKSPACE_TAB_BAR_HEIGHT: Pixels = px(36.);
const CONDR_ICON_PATHS: [&str; 2] = ["icons/circle.svg", "icons/circle-alert.svg"];

struct CondrAssets {
    base: Assets,
}

impl CondrAssets {
    fn new() -> Self {
        Self {
            base: Assets::new(""),
        }
    }
}

impl AssetSource for CondrAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            "icons/circle.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/circle.svg"
            )))),
            "icons/circle-alert.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/circle-alert.svg"
            )))),
            _ => self.base.load(path),
        }
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut assets = self.base.list(path)?;
        assets.extend(
            CONDR_ICON_PATHS
                .into_iter()
                .filter(|asset| asset.starts_with(path))
                .map(SharedString::from),
        );
        Ok(assets)
    }
}

fn default_worktree_branch(workspace_name: &str) -> String {
    let slug = workspace_name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_') {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>();
    let slug = slug.trim_matches('-');
    format!("worktree/{}", if slug.is_empty() { "change" } else { slug })
}

fn default_window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::centered(DEFAULT_WINDOW_SIZE, cx)),
        ..Default::default()
    }
}

struct ConnectionResult {
    key: ConnectionKey,
    generation: u64,
    endpoint: Endpoint,
    result: Result<ClientConnection, String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConnectionStatus {
    Connecting,
    Connected,
    Disconnected,
}

struct ServerConnection {
    key: ConnectionKey,
    label: String,
    endpoint: Endpoint,
    status: ConnectionStatus,
    server_id: Option<ServerId>,
    runtime_epoch: Option<RuntimeEpoch>,
    session_id: Option<SessionId>,
    sequence: u64,
    snapshot: SessionSnapshot,
    terminals: HashMap<PaneId, PaneTerminalSnapshot>,
    agents: HashMap<PaneId, AgentSnapshot>,
    agent_trackers: HashMap<PaneId, AgentTracker>,
    workspace_git: HashMap<WorkspaceId, WorkspaceGitSnapshot>,
    zoomed_panes: HashSet<PaneId>,
    io: Option<ClientIo>,
    connect_generation: u64,
    controlling: bool,
    subscribed: bool,
    subscription_pending: bool,
    control_retry_attempts: u8,
    control_retry_scheduled: bool,
    bootstrap_resync_session_id: Option<SessionId>,
    error: Option<String>,
    next_layout_request_id: u64,
}

struct BootstrapApplication {
    rebuild: bool,
    resubscribe: bool,
    reacquire_control: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PendingWorkspaceSelection {
    connection_key: ConnectionKey,
    workspace_id: WorkspaceId,
    pane_id: Option<PaneId>,
    connect_generation: u64,
    server_id: ServerId,
    runtime_epoch: RuntimeEpoch,
    session_id: SessionId,
    request_id: u64,
    applied_sequence: Option<u64>,
}

impl PendingWorkspaceSelection {
    fn belongs_to(self, connection: &ServerConnection) -> bool {
        self.connection_key == connection.key
            && self.connect_generation == connection.connect_generation
            && Some(self.server_id) == connection.server_id
            && Some(self.runtime_epoch) == connection.runtime_epoch
            && Some(self.session_id) == connection.session_id
    }
}

impl ServerConnection {
    fn new(key: ConnectionKey, label: String, endpoint: Endpoint) -> Self {
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
            agents: HashMap::new(),
            agent_trackers: HashMap::new(),
            workspace_git: HashMap::new(),
            zoomed_panes: HashSet::new(),
            io: None,
            connect_generation: 0,
            controlling: false,
            subscribed: false,
            subscription_pending: false,
            control_retry_attempts: 0,
            control_retry_scheduled: false,
            bootstrap_resync_session_id: None,
            error: None,
            next_layout_request_id: 1,
        }
    }

    fn can_mutate(&self) -> bool {
        self.is_synchronized() && self.controlling
    }

    fn reset_sync_state(&mut self) {
        self.controlling = false;
        self.subscribed = false;
        self.subscription_pending = false;
        self.control_retry_attempts = 0;
        self.control_retry_scheduled = false;
        self.bootstrap_resync_session_id = None;
    }

    fn is_synchronized(&self) -> bool {
        self.status == ConnectionStatus::Connected
            && self.subscribed
            && self.bootstrap_resync_session_id.is_none()
    }

    fn apply_bootstrap(&mut self, bootstrap: SessionBootstrap) -> BootstrapApplication {
        let previous_layout = self.dock_projection();
        let authority_changed = self.server_id != Some(bootstrap.server_id)
            || self.runtime_epoch != Some(bootstrap.runtime_epoch)
            || self.session_id != Some(bootstrap.session_id);
        let resubscribe = self.bootstrap_resync_session_id.is_some() && !self.subscribed;
        if authority_changed {
            self.controlling = false;
            self.subscribed = false;
            self.subscription_pending = false;
            self.control_retry_attempts = 0;
            self.control_retry_scheduled = false;
            self.agent_trackers.clear();
        }
        self.server_id = Some(bootstrap.server_id);
        self.runtime_epoch = Some(bootstrap.runtime_epoch);
        self.session_id = Some(bootstrap.session_id);
        self.sequence = bootstrap.sequence;
        self.snapshot = bootstrap.snapshot;
        self.terminals = bootstrap
            .terminals
            .into_iter()
            .map(|terminal| (terminal.pane_id, terminal))
            .collect();
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
        self.workspace_git = bootstrap
            .workspace_git
            .into_iter()
            .map(|git| (git.workspace_id, git))
            .collect();
        self.zoomed_panes = bootstrap.zoomed_panes.into_iter().collect();
        self.status = ConnectionStatus::Connected;
        self.bootstrap_resync_session_id = None;
        self.error = None;
        BootstrapApplication {
            rebuild: previous_layout != self.dock_projection(),
            resubscribe: resubscribe || authority_changed,
            reacquire_control: authority_changed,
        }
    }

    fn dock_projection(&self) -> Option<PaneLayout> {
        let session = Session::restore(self.snapshot.clone()).ok()?;
        let tab = session.active_workspace()?.active_tab();
        self.zoomed_panes
            .iter()
            .copied()
            .find(|pane_id| tab.panes().iter().any(|pane| pane.id() == *pane_id))
            .map(PaneLayout::Pane)
            .or_else(|| Some(tab.layout().clone()))
    }

    fn send(&mut self, message: ClientMessage) {
        let failed = self
            .io
            .as_ref()
            .is_none_or(|io| io.outgoing.send(message).is_err());
        if failed {
            self.status = ConnectionStatus::Disconnected;
            self.controlling = false;
            self.subscribed = false;
            self.subscription_pending = false;
            self.bootstrap_resync_session_id = None;
            self.error = Some("Disconnected from condr-server".into());
            self.io = None;
        }
    }

    fn subscribe(&mut self) {
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

    fn request_snapshot(&mut self) -> bool {
        let Some(session_id) = self.session_id else {
            return false;
        };
        self.request_snapshot_for(session_id)
    }

    fn request_snapshot_for(&mut self, session_id: SessionId) -> bool {
        if self.bootstrap_resync_session_id == Some(session_id) {
            return false;
        }
        self.bootstrap_resync_session_id = Some(session_id);
        self.send(ClientMessage::SnapshotRequest { session_id });
        true
    }

    fn recover_rejected_snapshot(
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
        self.error = Some(reason);
        self.request_snapshot_for(authoritative_session_id)
    }

    fn recover_rejected_subscription(
        &mut self,
        server_id: ServerId,
        authoritative_session_id: SessionId,
    ) -> bool {
        if !self.subscription_pending || self.server_id != Some(server_id) {
            return false;
        }
        self.subscription_pending = false;
        self.subscribed = false;
        self.request_snapshot_for(authoritative_session_id);
        true
    }
}

pub(crate) struct Condr {
    client_config_path: Option<PathBuf>,
    connections: Vec<ServerConnection>,
    active_connection: ConnectionKey,
    next_connection_key: ConnectionKey,
    connect_results_tx: async_channel::Sender<ConnectionResult>,
    _connect_results_task: Task<()>,
    dock_surfaces: HashMap<DockSurfaceKey, DockSurface>,
    active_dock_surface: Option<DockSurfaceKey>,
    #[cfg(feature = "test-support")]
    dock_rebuild_count: usize,
    panels: HashMap<(ConnectionKey, PaneId), Entity<TerminalPanel>>,
    target_pane: Option<(ConnectionKey, PaneId)>,
    pending_workspace_selections: HashMap<ConnectionKey, PendingWorkspaceSelection>,
    pending_presentation_request: Option<(ConnectionKey, u64)>,
    workspace_size: Size<Pixels>,
    focus_handle: FocusHandle,
    terminal_selection: Option<LocalTerminalSelection>,
    pending_sizes: HashMap<(ConnectionKey, PaneId), TerminalSize>,
    terminal_geometry: HashMap<(ConnectionKey, PaneId), TerminalGeometry>,
    marked_text: Option<String>,
    app_error: Option<String>,
}

fn accepted_text_input(value: String, trim_value: bool) -> Option<String> {
    if value.trim().is_empty() {
        return None;
    }
    Some(if trim_value {
        value.trim().to_owned()
    } else {
        value
    })
}

impl Condr {
    fn new(
        endpoint: Endpoint,
        initial: Result<ClientConnection, String>,
        client_config_path: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (connect_results_tx, connect_results_rx) =
            async_channel::bounded(CONNECTION_RESULT_BUFFER_CAPACITY);
        let mut connection = ServerConnection::new(1, "Local".into(), endpoint);
        if let Err(error) = Self::install_connection(&mut connection, initial, window, cx) {
            connection.status = ConnectionStatus::Disconnected;
            connection.error = Some(error);
        }

        let (saved_servers, config_error) = client_config_path
            .as_deref()
            .map_or_else(|| Ok(Vec::new()), config::load_servers)
            .map_or_else(
                |error| (Vec::new(), Some(format!("Failed to load config: {error}"))),
                |servers| (servers, None),
            );
        let mut connections = vec![connection];
        for (index, server) in saved_servers.into_iter().enumerate() {
            connections.push(ServerConnection::new(
                index as u64 + 2,
                server.name,
                Endpoint::tcp(server.address),
            ));
        }
        let next_connection_key = connections.len() as u64 + 1;

        let mut this = Self {
            client_config_path,
            connections,
            active_connection: 1,
            next_connection_key,
            connect_results_tx,
            _connect_results_task: Task::ready(()),
            dock_surfaces: HashMap::new(),
            active_dock_surface: None,
            #[cfg(feature = "test-support")]
            dock_rebuild_count: 0,
            panels: HashMap::new(),
            target_pane: None,
            pending_workspace_selections: HashMap::new(),
            pending_presentation_request: None,
            workspace_size: size(
                (window.viewport_size().width - INITIAL_SIDEBAR_WIDTH).max(px(0.)),
                window.viewport_size().height,
            ),
            focus_handle: cx.focus_handle(),
            terminal_selection: None,
            pending_sizes: HashMap::new(),
            terminal_geometry: HashMap::new(),
            marked_text: None,
            app_error: config_error,
        };
        this.refresh_target_pane(1);
        this.acquire_and_subscribe(1);

        this._connect_results_task = cx.spawn_in(window, async move |owner, cx| {
            while let Ok(result) = connect_results_rx.recv().await {
                if owner
                    .update_in(cx, |this, window, cx| {
                        this.handle_connection_result(result, window, cx);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        for key in 2..this.next_connection_key {
            _ = this.start_connect(key);
        }

        let owner = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let _ = owner.update(cx, |this, cx| this.rebuild_dock(window, cx));
        });
        this
    }
}

#[cfg(test)]
mod tests;
