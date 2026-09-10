mod actions;
mod config;
mod connection;
mod dialogs;
mod dock;
mod events;
mod ime;
mod navigation;
mod presentation;
mod server_connection;
mod server_management;
mod settings;
mod sidebar;
mod startup;
mod terminal_input;
mod terminal_panel;
mod workspace;

use crate::assets::{APP_LOGO, CondrAssets};
use crate::terminal_element::{
    HoveredTerminalLink, TerminalElement, TerminalElementProps, TerminalPalette,
    TerminalRenderCache, link_at,
};
use condr_core::agent_hooks::{HooksAction, HooksReport, HooksState};
use condr_core::protocol::{
    AgentCommand, AgentResponse, BootstrapAssembler, BootstrapHeader, ClientMessage, LayoutCommand,
    MAX_CHUNK_PAYLOAD_SIZE, MAX_CHUNKED_RECORD_SIZE, PaneTerminalFrame, PaneTerminalSnapshot,
    RuntimeEpoch, ServerAdminCommand, ServerAdminResponse, ServerClientInfo, ServerId,
    ServerMessage, ServerSettings, SessionBootstrap, SessionEvent, SessionId, TerminalFrameBatch,
    TerminalFrameChunk, WorkspaceGitSnapshot, decode_pane_terminal_frame, relative_age,
};
use condr_core::{
    AgentDisplayState, AgentKind, AgentSnapshot, AgentState, AgentTracker, PaneDirection, PaneId,
    PaneLayout, Session, SessionSnapshot, SplitDirection, TabId, TerminalCellRun, TerminalCommand,
    TerminalCursor, TerminalHyperlinkBudget, TerminalKey, TerminalModifiers, TerminalMouseButton,
    TerminalMouseEvent, TerminalMouseTracking, TerminalPosition, TerminalSelection,
    TerminalSelectionUnit, TerminalSize, TerminalViewDelta, TerminalViewFrame, WorkspaceId,
};
use condr_server::{
    ClientConnection, ConnectionCancellation, Endpoint, ServerConfig, StaticKey, TcpEndpoint,
};
use connection::*;
#[cfg(test)]
use dialogs::accepted_text_input;
use dock::*;
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
use ime::TerminalComposition;
use presentation::*;
use server_connection::*;
use settings::{
    Appearance, SettingsWindow, TerminalFont, apply_appearance, apply_terminal_color_scheme,
    apply_terminal_font, sync_theme_with_system,
};
#[cfg(all(test, feature = "test-support"))]
use settings::{
    SettingsTab, color_scheme_is_dirty, reset_color_scheme, select_appearance, select_server_shell,
    select_settings_server, select_settings_tab, select_terminal_font_family,
    select_terminal_font_size, selected_appearance, server_listen, server_shell, set_server_listen,
    step_terminal_font_size, terminal_font_family, terminal_font_size,
};
#[cfg(test)]
use sidebar::*;
use startup::{connect_to_server, fixed_shortcut};
#[cfg(test)]
use startup::{connect_to_server_with, lock_exclusively, single_instance_lock_path};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;
use terminal_input::{
    LocalTerminalSelection, ReportedTerminalMouse, ReportedTerminalMouseMotion, TerminalGeometry,
};
#[cfg(test)]
use terminal_input::{
    TerminalClipboardShortcut, should_defer_to_character_input, terminal_clipboard_shortcut,
};
use terminal_panel::TerminalPanel;
use workspace::default_worktree_branch;

pub(crate) use startup::run;

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

#[derive(Clone, Debug, PartialEq, Action)]
#[action(namespace = condr, no_json)]
struct ActivateTab {
    index: usize,
}

pub(crate) type ConnectionKey = u64;

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

/// The drag payload of the sidebar resize handle; the shell tracks its moves.
#[derive(Clone)]
struct DraggedSidebar;

fn default_window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::centered(DEFAULT_WINDOW_SIZE, cx)),
        // The window draws its own title bar and owns dragging.
        ..crate::assets::window_options()
    }
}

pub(crate) struct Condr {
    client_config_path: Option<PathBuf>,
    config_save: Option<Task<()>>,
    /// This device's static key for TCP Servers, kept beside `config.toml`. `None` when
    /// it could not be loaded or created; TCP Servers are then unavailable rather than
    /// reached with a key an attacker could predict.
    device_key: Option<StaticKey>,
    /// A failed load must never turn the missing saved list into an empty writeback.
    servers_error: Option<String>,
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
    sidebar_workspace_open: HashMap<(ConnectionKey, WorkspaceId), Entity<bool>>,
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
            servers_error,
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
            servers_error,
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
            sidebar_workspace_open: HashMap::new(),
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
        this.sync_sidebar_workspace_open(cx);
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
