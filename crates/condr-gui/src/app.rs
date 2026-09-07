use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use condr_core::agent_hooks::{HooksAction, HooksReport, HooksState};
use condr_core::protocol::{
    AgentCommand, AgentResponse, BootstrapAssembler, BootstrapHeader, ClientMessage, LayoutCommand,
    MAX_CHUNK_PAYLOAD_SIZE, MAX_CHUNKED_RECORD_SIZE, PaneTerminalFrame, PaneTerminalSnapshot,
    RuntimeEpoch, ServerId, ServerMessage, ServerSettings, SessionBootstrap, SessionEvent,
    SessionId, TerminalFrameBatch, TerminalFrameChunk, WorkspaceGitSnapshot,
    decode_pane_terminal_frame,
};
use condr_core::{
    AgentDisplayState, AgentKind, AgentSnapshot, AgentState, AgentTracker, PaneDirection, PaneId,
    PaneLayout, Session, SessionSnapshot, SplitDirection, TabId, TerminalCellRun, TerminalCommand,
    TerminalCursor, TerminalHyperlinkBudget, TerminalKey, TerminalModifiers, TerminalMouseButton,
    TerminalMouseEvent, TerminalMouseTracking, TerminalPosition, TerminalSelection, TerminalSize,
    TerminalViewDelta, TerminalViewFrame, WorkspaceId,
};
use condr_server::{ClientConnection, Endpoint, ServerConfig, StaticKey, TcpEndpoint};
use gpui_kit::assets::Assets;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::{Cancel, Confirm, DialogButtonProps, DialogFooter};
use gpui_kit::component::dock::{
    BasePanel, DockArea, DockAreaRenderer, DockEvent, DockLayout, PanelEvent, PanelInfo,
    PanelState, TabGroupRenderer, TilesRenderer,
};
use gpui_kit::component::input::{Editor, EditorState, Input, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::select::{SearchableVec, Select, SelectEvent, SelectState};
use gpui_kit::component::setting::{
    SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::component::sidebar::{Sidebar, SidebarCollapsible, SidebarItem};
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{
    ActiveTheme as _, Collapsible, Disableable as _, ElementExt as _, Icon, IconName, IconNamed,
    IndexPath, Root, Selectable as _, Sizable as _, StyledExt as _, TitleBar, WindowExt as _,
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::terminal_element::{
    HoveredTerminalLink, TerminalElement, TerminalElementProps, TerminalPalette,
    TerminalRenderCache, link_at,
};

mod actions;
mod config;
mod connection;
mod dialogs;
mod dock;
mod events;
mod navigation;
mod settings;
mod sidebar;
mod startup;
mod terminal_input;
mod workspace;

use connection::*;
use dock::*;
use settings::{
    Appearance, SettingsWindow, TerminalFont, apply_appearance, apply_terminal_color_scheme,
    apply_terminal_font, sync_theme_with_system,
};
#[cfg(all(test, feature = "test-support"))]
use settings::{
    SettingsTab, color_scheme_is_dirty, reset_color_scheme, select_appearance, select_server_shell,
    select_settings_server, select_settings_tab, select_terminal_font_family,
    select_terminal_font_size, selected_appearance, server_shell, step_terminal_font_size,
    terminal_font_family, terminal_font_size,
};
#[cfg(test)]
use sidebar::*;
pub(crate) use startup::run;
use startup::{connect_to_server, fixed_shortcut};
#[cfg(test)]
use startup::{connect_to_server_with, lock_exclusively, single_instance_lock_path};
#[cfg(test)]
use terminal_input::{
    TerminalClipboardShortcut, should_defer_to_character_input, terminal_clipboard_shortcut,
};

actions!(
    condr,
    [
        AddServer,
        ReconnectServer,
        NewWorkspace,
        NewTab,
        OpenSettings,
        RenameWorkspace,
        RenameTab,
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
        ToggleZoom,
        TerminalTab,
        TerminalBackTab
    ]
);

pub(crate) type ConnectionKey = u64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct TerminalComposition {
    pub(super) connection_key: ConnectionKey,
    pub(super) pane_id: PaneId,
    pub(super) text: String,
    pub(super) selected_range: Range<usize>,
}

/// A Pane's terminal as this Client holds it. The view is shared with the Panel that
/// paints it, so a frame redraw clones a pointer, not the cell grid; frames mutate it in
/// place through `Arc::make_mut` once the previous frame's element is gone.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ClientTerminal {
    pub(super) view: Arc<condr_core::TerminalView>,
    pub(super) exited: bool,
}

impl From<PaneTerminalSnapshot> for ClientTerminal {
    fn from(snapshot: PaneTerminalSnapshot) -> Self {
        Self {
            view: Arc::new(snapshot.view),
            exited: snapshot.exited,
        }
    }
}

const DEFAULT_WINDOW_SIZE: Size<Pixels> = size(px(1280.0), px(720.0));
const CONNECTION_RESULT_BUFFER_CAPACITY: usize = 16;
const SERVER_EVENT_BUFFER_CAPACITY: usize = 256;
const CONTROL_RETRY_DELAY: Duration = Duration::from_millis(50);
/// Retries double up to this while another client holds control; they never give up
/// while the connection stays up, so a controller that leaves seconds later is noticed.
const MAX_CONTROL_RETRY_DELAY: Duration = Duration::from_secs(2);
const CONTROL_BUSY_REASON: &str = "another client controls this Session";
const ACTIVE_PANE_BORDER_RGB: u32 = 0x0078d4;
const INITIAL_SIDEBAR_WIDTH: Pixels = px(240.);
const MIN_SIDEBAR_WIDTH: Pixels = px(150.);
const MAX_SIDEBAR_WIDTH: Pixels = px(360.);
const SIDEBAR_RESIZE_HANDLE_WIDTH: Pixels = px(6.);
const WORKSPACE_TAB_BAR_HEIGHT: Pixels = px(36.);
const CONDR_ICON_PATHS: [&str; 7] = [
    "icons/circle.svg",
    "icons/circle-filled.svg",
    "icons/circle-alert.svg",
    "icons/server-plus.svg",
    "icons/claude.svg",
    "icons/codex.svg",
    "icons/opencode.svg",
];

/// The drag payload of the sidebar resize handle; the shell tracks its moves.
#[derive(Clone)]
struct DraggedSidebar;

struct CondrAssets {
    base: Assets,
}

impl CondrAssets {
    fn new() -> Self {
        Self { base: Assets }
    }
}

impl AssetSource for CondrAssets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        match path {
            "icons/circle.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/circle.svg"
            )))),
            "icons/circle-filled.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/circle-filled.svg"
            )))),
            "icons/circle-alert.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/circle-alert.svg"
            )))),
            "icons/server-plus.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/server-plus.svg"
            )))),
            "icons/claude.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/claude.svg"
            )))),
            "icons/codex.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/codex.svg"
            )))),
            "icons/opencode.svg" => Ok(Some(Cow::Borrowed(include_bytes!(
                "../assets/icons/opencode.svg"
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
        // The window draws its own title bar and owns dragging.
        ..TitleBar::window_options()
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
    terminals: HashMap<PaneId, ClientTerminal>,
    terminal_titles: HashMap<PaneId, String>,
    terminal_hyperlinks: HashMap<PaneId, TerminalHyperlinkBudget>,
    agents: HashMap<PaneId, AgentSnapshot>,
    agent_trackers: HashMap<PaneId, AgentTracker>,
    /// Panes that rang BEL while not focused; cleared when the Terminal gains focus.
    attention: HashSet<PaneId>,
    workspace_git: HashMap<WorkspaceId, WorkspaceGitSnapshot>,
    zoomed_panes: HashSet<PaneId>,
    /// Server-owned preferences from the Bootstrap, kept current by events.
    settings: ServerSettings,
    /// The state of each agent's status hooks on the Server's machine, as last
    /// reported; requested when Settings shows this Server and after every action.
    hooks: Vec<HooksReport>,
    /// Why the last hooks request failed, until the next report.
    hooks_error: Option<String>,
    io: Option<ClientIo>,
    connect_generation: u64,
    controlling: bool,
    subscribed: bool,
    subscription_pending: bool,
    control_retry_attempts: u8,
    control_retry_scheduled: bool,
    bootstrap_resync_session_id: Option<SessionId>,
    /// Set by a subscription rejection: the next Bootstrap must re-acquire control and drop
    /// pending layout projections, because responses may have been lost to writer lag.
    reacquire_after_bootstrap: bool,
    error: Option<String>,
    next_layout_request_id: u64,
}

struct BootstrapApplication {
    rebuild: bool,
    resubscribe: bool,
    /// Re-send AcquireControl and drop pending layout projections.
    reacquire_control: bool,
    /// A different Server/runtime/Session: cached GUI state for the connection is stale.
    authority_changed: bool,
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
            terminal_titles: HashMap::new(),
            terminal_hyperlinks: HashMap::new(),
            agents: HashMap::new(),
            agent_trackers: HashMap::new(),
            attention: HashSet::new(),
            workspace_git: HashMap::new(),
            zoomed_panes: HashSet::new(),
            settings: ServerSettings::default(),
            hooks: Vec::new(),
            hooks_error: None,
            io: None,
            connect_generation: 0,
            controlling: false,
            subscribed: false,
            subscription_pending: false,
            control_retry_attempts: 0,
            control_retry_scheduled: false,
            bootstrap_resync_session_id: None,
            reacquire_after_bootstrap: false,
            error: None,
            next_layout_request_id: 1,
        }
    }

    fn can_mutate(&self) -> bool {
        self.is_synchronized() && self.controlling
    }

    fn reset_sync_state(&mut self) {
        self.controlling = false;
        self.attention.clear();
        self.subscribed = false;
        self.subscription_pending = false;
        self.control_retry_attempts = 0;
        self.control_retry_scheduled = false;
        self.bootstrap_resync_session_id = None;
        self.reacquire_after_bootstrap = false;
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
        BootstrapApplication {
            rebuild: previous_layout != self.dock_projection(),
            resubscribe: resubscribe || authority_changed,
            reacquire_control: authority_changed || rejected_recovery,
            authority_changed,
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

    /// Asks the Server to install, remove or report one agent's hooks on its machine.
    /// The reply comes back as an `AgentResult` and replaces that agent's row.
    fn send_agent_hooks(&mut self, agent: AgentKind, action: HooksAction) {
        let (Some(server_id), Some(session_id)) = (self.server_id, self.session_id) else {
            return;
        };
        self.send(ClientMessage::Agent {
            server_id,
            session_id,
            command: AgentCommand::Hooks { agent, action },
        });
    }

    fn send(&mut self, message: ClientMessage) {
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
            self.error = Some("Disconnected from Server".into());
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

pub(crate) struct Condr {
    client_config_path: Option<PathBuf>,
    config_save: Option<Task<()>>,
    /// This device's static key for TCP Servers, kept beside `config.toml`. `None` when
    /// it could not be loaded or created; TCP Servers are then unavailable rather than
    /// reached with a key an attacker could predict.
    device_key: Option<StaticKey>,
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
    hovered_link: Option<(ConnectionKey, PaneId, HoveredTerminalLink)>,
    pressed_terminal_link: Option<(ConnectionKey, PaneId, HoveredTerminalLink)>,
    terminal_mouse_capture: Option<ReportedTerminalMouse>,
    last_terminal_mouse_motion: Option<ReportedTerminalMouseMotion>,
    focused_terminal: Option<(ConnectionKey, PaneId)>,
    reported_terminal_focus: Option<(ConnectionKey, PaneId)>,
    pending_sizes: HashMap<(ConnectionKey, PaneId), TerminalSize>,
    terminal_geometry: HashMap<(ConnectionKey, PaneId), TerminalGeometry>,
    terminal_composition: Option<TerminalComposition>,
    window_handle: AnyWindowHandle,
    appearance: Appearance,
    fps_monitor: bool,
    /// Absolute, as Zed keeps its dock sizes: a window resize never changes it,
    /// only dragging the handle does.
    sidebar_width: Pixels,
    terminal_font: TerminalFont,
    terminal_color_scheme: SharedString,
    settings_window: Option<WindowHandle<Root>>,
    settings_view: Option<WeakEntity<SettingsWindow>>,
    _settings_window_closed: Option<Subscription>,
    /// The pending debounced font save; replacing it cancels the previous one.
    _font_save: Option<Task<()>>,
    _shell_save: Option<Task<()>>,
    /// The Shell value waiting for its debounce, and the Server it belongs to.
    pending_shell: Option<(ConnectionKey, String)>,
    /// Where the Tab, Workspace or Server being dragged would land; drawn as a line.
    drop_target: Option<sidebar::DropTarget>,
    /// Flushes a pending font save when the app quits before the debounce elapses.
    _quit_subscription: Subscription,
    app_error: Option<String>,
    _window_activation_subscription: Subscription,
    _window_appearance_subscription: Subscription,
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
        config: config::LoadedConfig,
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

        let config::LoadedConfig {
            path: client_config_path,
            device_key,
            servers: saved_servers,
            error: config_error,
            appearance,
            fps_monitor,
            terminal_font,
            terminal_color_scheme,
        } = config;
        let mut connections = vec![connection];
        for (index, (name, endpoint)) in saved_servers.into_iter().enumerate() {
            connections.push(ServerConnection::new(index as u64 + 2, name, endpoint));
        }
        let next_connection_key = connections.len() as u64 + 1;

        let window_activation_subscription =
            cx.observe_window_activation(window, |this, window, cx| {
                this.sync_terminal_focus(window, cx);
            });
        let window_appearance_subscription =
            cx.observe_window_appearance(window, |this, window, cx| {
                if this.appearance == Appearance::System {
                    sync_theme_with_system(window, cx);
                    // The Settings window follows the system too.
                    cx.refresh_windows();
                }
            });
        apply_appearance(appearance, Some(window), cx);
        apply_terminal_font(&terminal_font, cx);
        apply_terminal_color_scheme(&terminal_color_scheme, cx);
        let quit_subscription = cx.on_app_quit(|this, cx| {
            // A change made less than a debounce before quitting is still saved.
            if this._font_save.is_some() {
                this.save_terminal_font(cx);
            }
            this.flush_server_shell();
            let pending = this.config_save.take();
            async move {
                if let Some(pending) = pending {
                    pending.await;
                }
            }
        });
        let mut this = Self {
            client_config_path,
            config_save: None,
            device_key,
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
            hovered_link: None,
            pressed_terminal_link: None,
            terminal_mouse_capture: None,
            last_terminal_mouse_motion: None,
            focused_terminal: None,
            reported_terminal_focus: None,
            pending_sizes: HashMap::new(),
            terminal_geometry: HashMap::new(),
            terminal_composition: None,
            window_handle: window.window_handle(),
            appearance,
            fps_monitor,
            sidebar_width: INITIAL_SIDEBAR_WIDTH,
            terminal_font,
            terminal_color_scheme,
            settings_window: None,
            settings_view: None,
            _settings_window_closed: None,
            _font_save: None,
            _shell_save: None,
            pending_shell: None,
            drop_target: None,
            _quit_subscription: quit_subscription,
            app_error: config_error,
            _window_activation_subscription: window_activation_subscription,
            _window_appearance_subscription: window_appearance_subscription,
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
