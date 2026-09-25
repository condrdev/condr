mod actions;
mod changes;
mod config;
mod connection;
mod connection_status;
mod dialogs;
mod directory_browser;
mod dock;
mod events;
pub(crate) mod file_icons;
mod files;
mod gui_state;
mod ime;
mod navigation;
mod notifications;
mod open_in;
mod power;
mod presentation;
mod server_connection;
mod server_management;
mod settings;
mod sidebar;
mod startup;
mod terminal_input;
mod terminal_panel;
mod updates;
mod workspace;

use crate::assets::{APP_LOGO, CondrAssets};

/// Where the source lives and where the docs start; the About page and the Welcome
/// page both point there.
const REPOSITORY_URL: &str = "https://github.com/condrdev/condr";
const DOCS_URL: &str = "https://condr.dev/docs/getting-started";
use crate::terminal_element::{
    HoveredTerminalLink, TerminalElement, TerminalElementProps, TerminalPalette,
    TerminalRenderCache, link_at,
};
use changes::*;
use condr_core::agent_hooks::{HooksAction, HooksReport, HooksState};
use condr_core::protocol::{
    AgentCommand, AgentResponse, BootstrapAssembler, BootstrapHeader, ClientMessage, LayoutCommand,
    LayoutResult, MAX_CHUNK_PAYLOAD_SIZE, MAX_CHUNKED_RECORD_SIZE, PaneTerminalFrame,
    PaneTerminalSnapshot, RuntimeEpoch, ServerAdminCommand, ServerAdminResponse, ServerClientInfo,
    ServerId, ServerLogRecord, ServerMessage, ServerSettings, SessionBootstrap, SessionEvent,
    SessionId, TerminalFrameBatch, TerminalFrameChunk, UnknownMessage, WorkspaceGitSnapshot,
    decode_pane_terminal_frame, relative_age, uptime_text,
};
use condr_core::{
    AgentDisplayState, AgentKind, AgentSnapshot, AgentState, AgentTracker, BrowsedDirectory,
    DirectoryListing, FileContent, FileDiff, PaneDirection, PaneId, PaneLayout, Session,
    SessionSnapshot, SplitDirection, Tab, TabId, TerminalCellRun, TerminalCommand, TerminalCursor,
    TerminalHyperlinkBudget, TerminalKey, TerminalModifiers, TerminalMouseButton,
    TerminalMouseEvent, TerminalMouseTracking, TerminalPosition, TerminalSelection,
    TerminalSelectionUnit, TerminalSize, TerminalViewDelta, TerminalViewFrame, Workspace,
    WorkspaceId,
};
use condr_server::{
    ClientConnection, ConnectionCancellation, DeviceKey, Endpoint, ServerConfig, TcpEndpoint,
};
use connection::*;
#[cfg(test)]
use dialogs::accepted_text_input;
use directory_browser::{DirectoryBrowser, browser_field, render_directory_browser};
use dock::*;
use files::*;
use gpui_kit::component::badge::Badge;
use gpui_kit::component::button::{Button, ButtonVariant, ButtonVariants as _};
use gpui_kit::component::dialog::{Cancel, Confirm, DialogButtonProps, DialogFooter};
use gpui_kit::component::dock::{
    AnyDrag, BasePanel, DockArea, DockAreaRenderer, DockEvent, DockLayout, DockPlacement,
    DropTarget as DockDropTarget, PaneRef, PanelEvent, PanelId, PanelInfo, PanelState,
    TabGroupRenderer, TilesRenderer,
};
use gpui_kit::component::input::{Editor, EditorState, Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::select::{SearchableVec, Select, SelectEvent, SelectState};
use gpui_kit::component::separator::Separator;
use gpui_kit::component::setting::{
    SelectIndex, SettingField, SettingGroup, SettingItem, SettingPage, Settings,
};
use gpui_kit::component::sidebar::{Sidebar, SidebarCollapsible, SidebarItem};
use gpui_kit::component::spinner::Spinner;
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
use relative_path::{RelativePath, RelativePathBuf};
use server_connection::*;
use settings::{
    Appearance, SHORTCUT_CONTEXT, SettingsWindow, TerminalFont, apply_appearance,
    apply_terminal_color_scheme, apply_terminal_font, sync_theme_with_system,
};
#[cfg(all(test, feature = "test-support"))]
use settings::{
    SettingsTab, color_scheme_is_dirty, reset_color_scheme, select_appearance, select_server_shell,
    select_settings_server, select_settings_server_page, select_settings_tab,
    select_terminal_font_family, select_terminal_font_size, selected_appearance, server_listen,
    server_shell, set_server_listen, step_terminal_font_size, terminal_font_family,
    terminal_font_size,
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
use std::time::{Duration, Instant};
use terminal_input::{
    LocalTerminalSelection, ReportedTerminalMouse, ReportedTerminalMouseMotion, TerminalGeometry,
};
#[cfg(test)]
use terminal_input::{
    TerminalClipboardShortcut, should_defer_to_character_input, terminal_clipboard_shortcut,
};
use terminal_panel::TerminalPanel;
use updates::{UpdateChannel, UpdateState};
use workspace::{default_worktree_branch, title_bar};

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
        ToggleSidebar,
        ToggleChanges,
        NextWorkspace,
        PreviousWorkspace,
        TerminalTab,
        TerminalBackTab,
        QuitApp,
        HideApp,
        HideOtherApps,
        ShowAllApps,
        MinimizeWindow,
        ZoomWindow,
        ToggleFullScreen
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
/// How often the GUI retries the Server after asking it to restart, and for how long;
/// `condr server restart` itself waits up to 30 s for the old process to let go.
const RESTART_RECONNECT_DELAY: Duration = Duration::from_millis(500);
/// How long an unexpected drop may take to heal before the Workspace says "Reconnecting";
/// a blip that comes back inside this never shows.
const RECONNECT_GRACE: Duration = Duration::from_secs(2);
const RESTART_RECONNECT_TIMEOUT: Duration = Duration::from_secs(45);
/// Two messages from a newer protocol this close together mean the Server keeps sending
/// what this build cannot read: stop resynchronizing and say to update (ADR 0028).
const UNKNOWN_MESSAGE_WINDOW: Duration = Duration::from_secs(10);
const CONTROL_BUSY_REASON: &str = "another client controls this Session";
const ACTIVE_PANE_BORDER_RGB: u32 = 0x0078d4;
const INITIAL_SIDEBAR_WIDTH: Pixels = px(240.);
const MIN_SIDEBAR_WIDTH: Pixels = px(150.);
const MAX_SIDEBAR_WIDTH: Pixels = px(360.);
const COLLAPSED_SIDEBAR_WIDTH: Pixels = px(48.);
const SIDEBAR_RESIZE_HANDLE_WIDTH: Pixels = px(6.);
/// The main window's title bar hosts the Tab strip, so it takes the strip's old height
/// rather than the Kit's default; the Tabs keep the same room above and below.
const WORKSPACE_TITLE_BAR_HEIGHT: Pixels = px(36.);

/// The drag payload of the sidebar resize handle; the shell tracks its moves.
#[derive(Clone)]
struct DraggedSidebar;

fn default_window_options(state: Option<gui_state::WindowState>, cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(state.map_or_else(
            || WindowBounds::centered(DEFAULT_WINDOW_SIZE, cx),
            |state| state.window_bounds(DEFAULT_WINDOW_SIZE, cx),
        )),
        // The window draws its own title bar and owns dragging.
        ..crate::assets::window_options()
    }
}

pub(crate) struct Condr {
    client_config_path: Option<PathBuf>,
    config_save: Option<Task<()>>,
    /// This machine's Device key for TCP Servers, kept beside `config.toml`. `None` when
    /// it could not be loaded or created; TCP Servers are then unavailable rather than
    /// reached with a key an attacker could predict.
    device_key: Option<DeviceKey>,
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
    /// Whether agent completions in unwatched Panes go to the OS notification center.
    notifications: bool,
    /// Whether the machine is kept from sleeping and blanking while Condr runs.
    keep_awake: bool,
    /// Whether the update check runs on its own, every five hours (ADR 0029).
    auto_check_updates: bool,
    /// Which published builds the update check looks for.
    update_channel: UpdateChannel,
    /// What the last check found; a newer build puts a dot on the Settings button and
    /// is named on the About page.
    update_state: UpdateState,
    /// The Check button's request is in flight.
    checking_updates: bool,
    /// The five-hourly checks while `auto_check_updates` is on.
    _automatic_update_checks: Task<()>,
    /// The OS request behind `keep_awake`; dropping it lets the machine sleep again.
    /// Windows implements it with `SetThreadExecutionState`, which is per thread: it must
    /// be created and dropped on the GPUI main thread, never from a background task.
    _keep_awake: Option<keepawake::KeepAwake>,
    /// Absolute, as Zed keeps its dock sizes: a window resize never changes it,
    /// only dragging the handle does.
    sidebar_width: Pixels,
    sidebar_collapsed: bool,
    /// The Changes sidebar on the right (ADR 0017): the Workspaces showing it, and how
    /// wide. Per Workspace like `sidebar_workspace_open`, and like it not persisted.
    changes_open: HashSet<(ConnectionKey, WorkspaceId)>,
    changes_width: Pixels,
    collapsed_changes_sections: HashSet<ChangesSection>,
    /// Directories folded shut in the Changes tree, by connection, Workspace and
    /// repository path; Workspace ids repeat across Servers.
    collapsed_change_dirs: HashSet<(ConnectionKey, WorkspaceId, RelativePathBuf)>,
    /// The Diff Tabs' Editors, by connection and Tab; pruned with the Tabs.
    diff_editors: HashMap<(ConnectionKey, TabId), DiffEditor>,
    /// Diffs asked of a Server and not yet answered, so a redraw asks only once.
    pending_diffs: HashSet<(ConnectionKey, WorkspaceId, RelativePathBuf)>,
    /// Which view the right sidebar shows for each Workspace the user chose one for
    /// (ADR 0018); not persisted. Others open on Changes, or Files outside a repository.
    sidebar_views: HashMap<(ConnectionKey, WorkspaceId), SidebarView>,
    /// The terminal Tab each Workspace presented last: where "Insert Path into Terminal"
    /// sends its text once a viewer Tab has taken the Workspace's active slot.
    last_terminal_tabs: HashMap<(ConnectionKey, WorkspaceId), TabId>,
    /// Directories unfolded in the Files tree, by connection, Workspace and root-relative
    /// path.
    expanded_dirs: HashSet<(ConnectionKey, WorkspaceId, RelativePathBuf)>,
    /// The Preview Tabs' Editors, by connection and Tab; pruned with the Tabs.
    file_editors: HashMap<(ConnectionKey, TabId), FileEditor>,
    pending_directories: HashSet<(ConnectionKey, WorkspaceId, RelativePathBuf)>,
    pending_files: HashSet<(ConnectionKey, WorkspaceId, RelativePathBuf)>,
    /// The 0-based line the Preview Tab lands on once it shows this file: set by the Diff
    /// Tab's "Show File", applied by `sync_file_view` when the text is there, then cleared.
    /// Presentation only, so it never rides `ShowFile`.
    pending_file_line: Option<(ConnectionKey, WorkspaceId, RelativePathBuf, u32)>,
    /// What the open Root Directory dialog lists for a remote Device; replaced by the
    /// next such dialog, so a closed one is harmless.
    directory_browser: Option<DirectoryBrowser>,
    sidebar_workspace_open: HashMap<(ConnectionKey, WorkspaceId), Entity<bool>>,
    terminal_font: TerminalFont,
    terminal_color_scheme: SharedString,
    /// What "Open in" offers, once the startup scan has reported; `None` hides the button.
    open_targets: Option<Vec<open_in::OpenTarget>>,
    /// The Workspace whose "Open in" launch is in flight; its button shows a spinner.
    opening_workspace: Option<(ConnectionKey, WorkspaceId)>,
    /// The "Open in" target used last: the default for a project without its own choice.
    default_editor: Option<String>,
    custom_editors: Vec<open_in::CustomEditor>,
    /// Each project's "Open in" choice, keyed by its repository root.
    workspace_editors: BTreeMap<PathBuf, String>,
    settings_window: Option<WindowHandle<Root>>,
    settings_view: Option<WeakEntity<SettingsWindow>>,
    _settings_window_closed: Option<Subscription>,
    /// The pending debounced font save; replacing it cancels the previous one.
    _font_save: Option<Task<()>>,
    /// The Shell value waiting for its debounce, and the Server it belongs to.
    /// Where the Tab, Workspace or Server being dragged would land; drawn as a line.
    drop_target: Option<sidebar::DropTarget>,
    /// Flushes a pending font save when the app quits before the debounce elapses.
    _quit_subscription: Subscription,
    /// The most recent failure given to `report_error`, kept so tests can check it.
    pub(super) last_error: Option<String>,
    _window_activation_subscription: Subscription,
    _window_appearance_subscription: Subscription,
    _window_bounds_subscription: Subscription,
    /// The GUI state file (ADR 0023); `None` in tests that want no file.
    state_path: Option<PathBuf>,
    /// The state as read at startup: Servers not yet connected are restored from it when
    /// they bootstrap, and Servers never connected are written back from it unchanged.
    restored_state: gui_state::GuiState,
    /// The window's bounds as last observed, so a save needs no `Window`.
    window_state: Option<gui_state::WindowState>,
    /// A view change noticed where no context could schedule the save; `rebuild_dock`
    /// picks it up.
    state_dirty: bool,
    /// The debounce, then the write; awaited on quit like `config_save`.
    state_save: Option<Task<()>>,
}

impl Condr {
    fn new(
        endpoint: Endpoint,
        initial: Option<Result<ClientConnection, String>>,
        config: config::LoadedConfig,
        state: gui_state::LoadedState,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let gui_state::LoadedState {
            path: state_path,
            state: restored_state,
        } = state;
        let (connect_results_tx, connect_results_rx) =
            async_channel::bounded(CONNECTION_RESULT_BUFFER_CAPACITY);
        let mut connection = ServerConnection::new(1, "Local".into(), endpoint);
        // Without a ready connection the window opens first and connects like any
        // other device, so a slow or absent local Server never holds it back.
        let connect_local = initial.is_none();
        if let Some(initial) = initial
            && let Err(error) = Self::install_connection(&mut connection, initial, window, cx)
        {
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
            notifications,
            keep_awake,
            auto_check_updates,
            update_channel,
            terminal_font,
            terminal_color_scheme,
            default_editor,
            custom_editors,
            workspace_editors,
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
        let window_bounds_subscription = cx.observe_window_bounds(window, |this, window, cx| {
            this.window_state = Some(window.window_bounds().into());
            this.schedule_state_save(cx);
        });
        let quit_subscription = cx.on_app_quit(|this, cx| {
            // A change made less than a debounce before quitting is still saved.
            if this._font_save.is_some() {
                this.save_terminal_font(cx);
            }
            let pending = this.config_save.take();
            this.save_state_now(cx);
            let pending_state = this.state_save.take();
            async move {
                if let Some(pending) = pending {
                    pending.await;
                }
                if let Some(pending) = pending_state {
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
            workspace_size: size(
                (window.viewport_size().width - INITIAL_SIDEBAR_WIDTH).max(px(0.)),
                (window.viewport_size().height - WORKSPACE_TITLE_BAR_HEIGHT).max(px(0.)),
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
            notifications,
            keep_awake,
            auto_check_updates,
            update_channel,
            update_state: UpdateState::Unknown,
            checking_updates: false,
            _automatic_update_checks: Task::ready(()),
            _keep_awake: None,
            sidebar_width: restored_state
                .sidebar_width
                .map_or(INITIAL_SIDEBAR_WIDTH, |width| {
                    px(width).max(MIN_SIDEBAR_WIDTH).min(MAX_SIDEBAR_WIDTH)
                }),
            sidebar_collapsed: restored_state.sidebar_collapsed,
            changes_open: HashSet::new(),
            changes_width: restored_state
                .changes_width
                .map_or(INITIAL_CHANGES_WIDTH, |width| {
                    px(width).max(MIN_CHANGES_WIDTH).min(MAX_CHANGES_WIDTH)
                }),
            collapsed_changes_sections: HashSet::new(),
            collapsed_change_dirs: HashSet::new(),
            diff_editors: HashMap::new(),
            pending_diffs: HashSet::new(),
            sidebar_views: HashMap::new(),
            last_terminal_tabs: HashMap::new(),
            expanded_dirs: HashSet::new(),
            file_editors: HashMap::new(),
            pending_directories: HashSet::new(),
            pending_files: HashSet::new(),
            pending_file_line: None,
            directory_browser: None,
            sidebar_workspace_open: HashMap::new(),
            terminal_font,
            terminal_color_scheme,
            open_targets: None,
            opening_workspace: None,
            default_editor,
            custom_editors,
            workspace_editors,
            settings_window: None,
            settings_view: None,
            _settings_window_closed: None,
            _font_save: None,
            drop_target: None,
            _quit_subscription: quit_subscription,
            last_error: None,
            _window_activation_subscription: window_activation_subscription,
            _window_appearance_subscription: window_appearance_subscription,
            _window_bounds_subscription: window_bounds_subscription,
            state_path,
            window_state: Some(window.window_bounds().into()),
            restored_state,
            state_dirty: false,
            state_save: None,
        };
        this.sync_sidebar_workspace_open(cx);
        // A connection handed in ready has bootstrapped already, so restore it here.
        if this.connections[0].server_id.is_some() {
            this.restore_server_state(1, cx);
        }
        this.refresh_target_pane(1);
        this.acquire_and_subscribe(1);
        if let Some(error) = config_error {
            this.report_error(error, cx);
        }
        this.apply_keep_awake(cx);
        this.scan_open_targets(cx);
        if this.auto_check_updates {
            this.start_automatic_update_checks(updates::FIRST_CHECK_DELAY, cx);
        }

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

        if connect_local {
            _ = this.start_connect(1);
        }
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
