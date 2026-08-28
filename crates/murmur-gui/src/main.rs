mod terminal_element;

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::ops::Range;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::dialog::DialogButtonProps;
use gpui_component::dock::{
    BasePanel, DockArea, DockAreaRenderer, DockEvent, DockLayout, PanelEvent, PanelInfo,
    PanelState, TabGroupRenderer, TilesRenderer,
};
use gpui_component::input::{Input, InputState};
use gpui_component::menu::ContextMenuExt as _;
use gpui_component::resizable::{h_resizable, resizable_panel};
use gpui_component::sidebar::{
    Sidebar, SidebarCollapsible, SidebarFooter, SidebarGroup, SidebarHeader, SidebarMenu,
    SidebarMenuItem,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Root, Selectable as _, Sizable as _,
    StyledExt as _, WindowExt as _, h_flex, v_flex,
};
use gpui_component_assets::Assets;
use murmur_core::protocol::{
    ClientMessage, LayoutCommand, PaneTerminalSnapshot, RuntimeEpoch, ServerId, ServerMessage,
    SessionBootstrap, SessionEvent, SessionId,
};
use murmur_core::{
    PaneDirection, PaneId, PaneLayout, Session, SessionSnapshot, SplitDirection, TerminalCommand,
    TerminalKey, TerminalModifiers, TerminalPosition, TerminalScroll, TerminalSize, WorkspaceId,
};
use murmur_server::{ClientConnection, Endpoint, ServerConfig};

use crate::terminal_element::TerminalElement;

actions!(
    murmur,
    [
        AddServer,
        ReconnectServer,
        NewWorkspace,
        OpenFolder,
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

enum Incoming {
    Message(ServerMessage),
    Disconnected(String),
}

struct ClientIo {
    outgoing: mpsc::Sender<ClientMessage>,
    incoming: mpsc::Receiver<Incoming>,
}

impl ClientIo {
    fn start(connection: ClientConnection) -> std::io::Result<Self> {
        let mut reader = connection.into_stream();
        let mut writer = reader.try_clone()?;
        let (outgoing, outgoing_rx) = mpsc::channel();
        let (incoming_tx, incoming) = mpsc::channel();
        let writer_events = incoming_tx.clone();

        thread::Builder::new()
            .name("murmur-client-writer".into())
            .spawn(move || {
                while let Ok(message) = outgoing_rx.recv() {
                    if let Err(error) = murmur_core::protocol::write_message(&mut writer, &message)
                    {
                        let _ = writer_events.send(Incoming::Disconnected(error.to_string()));
                        break;
                    }
                }
            })?;
        thread::Builder::new()
            .name("murmur-client-reader".into())
            .spawn(move || {
                loop {
                    match murmur_core::protocol::read_message(&mut reader) {
                        Ok(message) => {
                            if incoming_tx.send(Incoming::Message(message)).is_err() {
                                break;
                            }
                        }
                        Err(error) => {
                            let _ = incoming_tx.send(Incoming::Disconnected(error.to_string()));
                            break;
                        }
                    }
                }
            })?;

        Ok(Self { outgoing, incoming })
    }
}

struct ConnectionResult {
    key: ConnectionKey,
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
    zoomed_panes: HashSet<PaneId>,
    io: Option<ClientIo>,
    controlling: bool,
    error: Option<String>,
}

impl ServerConnection {
    fn new(key: ConnectionKey, label: String, endpoint: Endpoint) -> Self {
        Self {
            key,
            label,
            endpoint,
            status: ConnectionStatus::Connecting,
            server_id: None,
            runtime_epoch: None,
            session_id: None,
            sequence: 0,
            snapshot: Session::new().snapshot(),
            terminals: HashMap::new(),
            zoomed_panes: HashSet::new(),
            io: None,
            controlling: false,
            error: None,
        }
    }

    fn can_mutate(&self) -> bool {
        self.status == ConnectionStatus::Connected && self.controlling
    }

    fn apply_bootstrap(&mut self, bootstrap: SessionBootstrap) {
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
        self.zoomed_panes = bootstrap.zoomed_panes.into_iter().collect();
        self.status = ConnectionStatus::Connected;
        self.error = None;
    }

    fn send(&mut self, message: ClientMessage) {
        let failed = self
            .io
            .as_ref()
            .is_none_or(|io| io.outgoing.send(message).is_err());
        if failed {
            self.status = ConnectionStatus::Disconnected;
            self.controlling = false;
            self.error = Some("Disconnected from murmur-server".into());
            self.io = None;
        }
    }
}

#[derive(Clone, Copy)]
struct TerminalGeometry {
    bounds: Bounds<Pixels>,
    cell_size: Size<Pixels>,
}

struct TerminalPanel {
    connection_key: ConnectionKey,
    pane_id: PaneId,
    owner: WeakEntity<Murmur>,
    focus_handle: FocusHandle,
}

struct MurmurDockRenderer;

impl DockAreaRenderer for MurmurDockRenderer {
    fn frame(&self, _: &mut Window, _: &mut App) -> Stateful<Div> {
        div()
            .id("murmur-dock-area")
            .size_full()
            .overflow_hidden()
            .flex()
            .flex_row()
    }

    fn center_frame(&self, _: &mut Window, _: &mut App) -> Stateful<Div> {
        div()
            .id("murmur-dock-center")
            .flex()
            .flex_1()
            .flex_col()
            .overflow_hidden()
    }

    fn split_frame(
        &self,
        node: gpui_component::dock::NodeId,
        _: Axis,
        _: &mut Window,
        _: &mut App,
    ) -> Stateful<Div> {
        div()
            .id(("murmur-dock-split", node.as_u64()))
            .size_full()
            .flex_1()
            .min_h(px(0.))
            .overflow_hidden()
    }

    fn tab_group_renderer(&self) -> Rc<dyn TabGroupRenderer> {
        Rc::new(MurmurTabGroupRenderer)
    }

    fn tiles_renderer(&self) -> Rc<dyn TilesRenderer> {
        Rc::new(MurmurTilesRenderer)
    }
}

struct MurmurTabGroupRenderer;

impl TabGroupRenderer for MurmurTabGroupRenderer {
    fn frame(
        &self,
        _: &gpui_component::dock::TabGroupContext,
        _: &mut Window,
        _: &mut App,
    ) -> Stateful<Div> {
        div()
            .id("murmur-tab-group")
            .size_full()
            .flex()
            .flex_col()
            .overflow_hidden()
    }

    fn content_frame(
        &self,
        _: &gpui_component::dock::TabGroupContext,
        _: &mut Window,
        _: &mut App,
    ) -> Stateful<Div> {
        div()
            .id("murmur-tab-content")
            .size_full()
            .flex_1()
            .min_h(px(0.))
            .overflow_hidden()
    }

    fn render_tab_bar(
        &self,
        _: &gpui_component::dock::TabGroupContext,
        _: &mut Window,
        _: &mut App,
    ) -> AnyElement {
        Empty.into_any_element()
    }
}

struct MurmurTilesRenderer;

impl TilesRenderer for MurmurTilesRenderer {
    fn frame(&self, _: &mut Window, _: &mut App) -> Stateful<Div> {
        div().id("murmur-tiles").size_full().overflow_hidden()
    }

    fn render_drag_bar(
        &self,
        _: &gpui_component::dock::TileContext,
        _: &mut Window,
        _: &mut App,
    ) -> AnyElement {
        Empty.into_any_element()
    }
}

impl TerminalPanel {
    fn new(
        connection_key: ConnectionKey,
        pane_id: PaneId,
        owner: WeakEntity<Murmur>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            connection_key,
            pane_id,
            owner,
            focus_handle: cx.focus_handle(),
        }
    }
}

impl EventEmitter<PanelEvent> for TerminalPanel {}

impl Focusable for TerminalPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl BasePanel for TerminalPanel {
    fn panel_name(&self) -> &'static str {
        "MurmurTerminal"
    }

    fn closable(&self, _: &App) -> bool {
        false
    }

    fn zoomable(&self, _: &App) -> bool {
        false
    }
}

impl Render for TerminalPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = self.owner.upgrade();
        let (terminal, active, controlling, marked_text) = owner
            .as_ref()
            .map(|owner| {
                let app = owner.read(cx);
                let active = app.target_pane == Some((self.connection_key, self.pane_id));
                (
                    app.terminal(self.connection_key, self.pane_id).cloned(),
                    active,
                    app.connection(self.connection_key)
                        .is_some_and(ServerConnection::can_mutate),
                    active.then(|| app.marked_text.clone()).flatten(),
                )
            })
            .unwrap_or((None, false, false, None));

        let key = self.connection_key;
        let pane_id = self.pane_id;
        let focus = self.focus_handle.clone();
        let click_owner = self.owner.clone();
        let right_click_owner = self.owner.clone();
        let body = div()
            .id(format!("terminal-pane-{key}-{}", pane_id.as_u64()))
            .key_context("Murmur")
            .track_focus(&self.focus_handle)
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                focus.focus(window, cx);
                let _ = click_owner.update(cx, |app, cx| app.select_pane(key, pane_id, cx));
            })
            .on_mouse_down(MouseButton::Right, move |_, _, cx| {
                let _ = right_click_owner.update(cx, |app, cx| {
                    app.set_target_pane(key, pane_id, cx);
                });
            })
            .size_full()
            .overflow_hidden()
            .p_1()
            .font_family(cx.theme().mono_font_family.clone())
            .text_size(cx.theme().mono_font_size)
            .line_height(relative(1.35))
            .border_1()
            .border_color(if active {
                cx.theme().ring
            } else {
                cx.theme().border
            })
            .child(if let (Some(owner), Some(terminal)) = (owner, terminal) {
                TerminalElement::new(
                    owner,
                    self.focus_handle.clone(),
                    key,
                    pane_id,
                    terminal.view,
                    marked_text,
                )
                .into_any_element()
            } else {
                div()
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("Terminal unavailable")
                    .into_any_element()
            });

        body.context_menu(move |menu, _, _| {
            menu.menu_with_enable("Split Right", Box::new(SplitRight), controlling)
                .menu_with_enable("Split Down", Box::new(SplitDown), controlling)
                .separator()
                .menu_with_enable("Swap Left", Box::new(SwapLeft), controlling)
                .menu_with_enable("Swap Right", Box::new(SwapRight), controlling)
                .menu_with_enable("Swap Up", Box::new(SwapUp), controlling)
                .menu_with_enable("Swap Down", Box::new(SwapDown), controlling)
                .separator()
                .menu_with_enable("Toggle Zoom", Box::new(ToggleZoom), controlling)
                .menu_with_enable("Close Pane", Box::new(ClosePane), controlling)
        })
    }
}

pub(crate) struct Murmur {
    connections: Vec<ServerConnection>,
    active_connection: ConnectionKey,
    next_connection_key: ConnectionKey,
    connect_results_tx: mpsc::Sender<ConnectionResult>,
    connect_results_rx: mpsc::Receiver<ConnectionResult>,
    dock_area: Entity<DockArea>,
    _dock_subscription: Subscription,
    rebuilding_dock: bool,
    panels: HashMap<(ConnectionKey, PaneId), Entity<TerminalPanel>>,
    target_pane: Option<(ConnectionKey, PaneId)>,
    focus_handle: FocusHandle,
    selecting_from: Option<(ConnectionKey, PaneId, TerminalPosition)>,
    pending_sizes: HashMap<(ConnectionKey, PaneId), TerminalSize>,
    terminal_geometry: HashMap<(ConnectionKey, PaneId), TerminalGeometry>,
    marked_text: Option<String>,
    app_error: Option<String>,
}

impl Murmur {
    fn new(
        endpoint: Endpoint,
        initial: Result<ClientConnection, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let dock_area = cx.new(|cx| {
            DockArea::new("murmur-workspace", None, window, cx)
                .with_renderer(Rc::new(MurmurDockRenderer))
        });
        let dock_subscription = cx.subscribe_in(
            &dock_area,
            window,
            |this, dock, event: &DockEvent, _, cx| {
                if matches!(event, DockEvent::LayoutChanged) {
                    this.on_dock_layout_changed(dock, cx);
                }
            },
        );
        let (connect_results_tx, connect_results_rx) = mpsc::channel();
        let mut connection = ServerConnection::new(1, "Local".into(), endpoint);
        if let Err(error) = Self::install_connection(&mut connection, initial) {
            connection.status = ConnectionStatus::Disconnected;
            connection.error = Some(error);
        }

        let mut this = Self {
            connections: vec![connection],
            active_connection: 1,
            next_connection_key: 2,
            connect_results_tx,
            connect_results_rx,
            dock_area,
            _dock_subscription: dock_subscription,
            rebuilding_dock: false,
            panels: HashMap::new(),
            target_pane: None,
            focus_handle: cx.focus_handle(),
            selecting_from: None,
            pending_sizes: HashMap::new(),
            terminal_geometry: HashMap::new(),
            marked_text: None,
            app_error: None,
        };
        this.acquire_and_subscribe(1);

        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                if this
                    .update_in(cx, |this, window, cx| {
                        if this.poll_connections(window, cx) {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let owner = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let _ = owner.update(cx, |this, cx| this.rebuild_dock(window, cx));
        });
        this
    }

    fn install_connection(
        connection: &mut ServerConnection,
        result: Result<ClientConnection, String>,
    ) -> Result<(), String> {
        let client = result?;
        let bootstrap = client.bootstrap().clone();
        let io = ClientIo::start(client).map_err(|error| error.to_string())?;
        connection.apply_bootstrap(bootstrap);
        connection.io = Some(io);
        connection.controlling = false;
        Ok(())
    }

    fn connection(&self, key: ConnectionKey) -> Option<&ServerConnection> {
        self.connections
            .iter()
            .find(|connection| connection.key == key)
    }

    fn connection_mut(&mut self, key: ConnectionKey) -> Option<&mut ServerConnection> {
        self.connections
            .iter_mut()
            .find(|connection| connection.key == key)
    }

    fn active_connection(&self) -> Option<&ServerConnection> {
        self.connection(self.active_connection)
    }

    fn active_session(&self) -> Option<Session> {
        Session::restore(self.active_connection()?.snapshot.clone()).ok()
    }

    fn terminal(
        &self,
        connection_key: ConnectionKey,
        pane_id: PaneId,
    ) -> Option<&PaneTerminalSnapshot> {
        self.connection(connection_key)?.terminals.get(&pane_id)
    }

    fn acquire_and_subscribe(&mut self, key: ConnectionKey) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let Some(session_id) = connection.session_id else {
            return;
        };
        connection.send(ClientMessage::AcquireControl { session_id });
        connection.send(ClientMessage::Subscribe {
            session_id,
            after_sequence: connection.sequence,
        });
    }

    fn start_connect(&mut self, key: ConnectionKey) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        connection.status = ConnectionStatus::Connecting;
        connection.controlling = false;
        connection.error = None;
        connection.io = None;
        let endpoint = connection.endpoint.clone();
        let sender = self.connect_results_tx.clone();
        thread::spawn(move || {
            let connected_endpoint = if matches!(endpoint, Endpoint::Local(_)) {
                murmur_server::ensure_local_server().unwrap_or(endpoint)
            } else {
                endpoint
            };
            let result = ClientConnection::connect(&connected_endpoint, "murmur-gui")
                .map_err(|error| error.to_string());
            let _ = sender.send(ConnectionResult {
                key,
                endpoint: connected_endpoint,
                result,
            });
        });
    }

    fn poll_connections(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        while let Ok(result) = self.connect_results_rx.try_recv() {
            let key = result.key;
            let installed = if let Some(connection) = self.connection_mut(key) {
                connection.endpoint = result.endpoint;
                match Self::install_connection(connection, result.result) {
                    Ok(()) => true,
                    Err(error) => {
                        connection.status = ConnectionStatus::Disconnected;
                        connection.error = Some(error);
                        false
                    }
                }
            } else {
                false
            };
            if installed {
                self.acquire_and_subscribe(key);
                self.refresh_target_pane(key);
                if key == self.active_connection {
                    self.rebuild_dock(window, cx);
                }
            }
            changed = true;
        }

        let mut incoming = Vec::new();
        for connection in &self.connections {
            if let Some(io) = &connection.io {
                while let Ok(message) = io.incoming.try_recv() {
                    incoming.push((connection.key, message));
                }
            }
        }
        for (key, message) in incoming {
            changed = true;
            if self.handle_incoming(key, message, cx) && key == self.active_connection {
                self.rebuild_dock(window, cx);
            }
        }
        changed
    }

    fn handle_incoming(
        &mut self,
        key: ConnectionKey,
        incoming: Incoming,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(index) = self
            .connections
            .iter()
            .position(|connection| connection.key == key)
        else {
            return false;
        };
        let message = match incoming {
            Incoming::Message(message) => message,
            Incoming::Disconnected(error) => {
                let connection = &mut self.connections[index];
                connection.status = ConnectionStatus::Disconnected;
                connection.controlling = false;
                connection.error = Some(format!("Server connection closed: {error}"));
                connection.io = None;
                return false;
            }
        };

        match message {
            ServerMessage::Bootstrap(bootstrap) => {
                self.connections[index].apply_bootstrap(bootstrap);
                self.refresh_target_pane(key);
                true
            }
            ServerMessage::Event {
                sequence, event, ..
            } => {
                self.connections[index].sequence = sequence;
                match event {
                    SessionEvent::LayoutChanged => {
                        if let Some(session_id) = self.connections[index].session_id {
                            self.connections[index]
                                .send(ClientMessage::SnapshotRequest { session_id });
                        }
                    }
                    SessionEvent::TerminalChanged { pane_id, view } => {
                        let pending_key = (key, pane_id);
                        if self.pending_sizes.get(&pending_key) == Some(&view.size) {
                            self.pending_sizes.remove(&pending_key);
                        }
                        self.connections[index].terminals.insert(
                            pane_id,
                            PaneTerminalSnapshot {
                                pane_id,
                                view,
                                exited: false,
                            },
                        );
                    }
                    SessionEvent::TerminalExited { pane_id, view } => {
                        self.connections[index].terminals.insert(
                            pane_id,
                            PaneTerminalSnapshot {
                                pane_id,
                                view,
                                exited: true,
                            },
                        );
                    }
                }
                false
            }
            ServerMessage::ControlGranted { .. } => {
                self.connections[index].controlling = true;
                self.connections[index].error = None;
                false
            }
            ServerMessage::ControlReleased { .. } => {
                self.connections[index].controlling = false;
                false
            }
            ServerMessage::ControlDenied { reason, .. }
            | ServerMessage::Error { message: reason } => {
                self.connections[index].error = Some(reason);
                false
            }
            ServerMessage::TerminalCopied { text, .. } => {
                if let Some(text) = text {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
                false
            }
            ServerMessage::ServerStopping => {
                let connection = &mut self.connections[index];
                connection.status = ConnectionStatus::Disconnected;
                connection.controlling = false;
                connection.error = Some("murmur-server stopped".into());
                connection.io = None;
                false
            }
            ServerMessage::Welcome { .. }
            | ServerMessage::Subscribed { .. }
            | ServerMessage::Pong { .. } => false,
        }
    }

    fn refresh_target_pane(&mut self, key: ConnectionKey) {
        let target = self.connection_focused_pane(key);
        if key == self.active_connection {
            self.target_pane = target.map(|pane_id| (key, pane_id));
        }
    }

    fn connection_focused_pane(&self, key: ConnectionKey) -> Option<PaneId> {
        self.connection(key)
            .and_then(|connection| Session::restore(connection.snapshot.clone()).ok())
            .and_then(|session| Some(session.active_workspace()?.active_tab().focused_pane().id()))
    }

    fn select_server(&mut self, key: ConnectionKey, window: &mut Window, cx: &mut Context<Self>) {
        if self.connection(key).is_none() {
            return;
        }
        self.active_connection = key;
        self.refresh_target_pane(key);
        self.rebuild_dock(window, cx);
        cx.notify();
    }

    fn select_workspace(&mut self, key: ConnectionKey, workspace_id: WorkspaceId) {
        self.active_connection = key;
        self.send_layout_to(key, LayoutCommand::ActivateWorkspace { workspace_id });
    }

    pub(crate) fn select_pane(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        if self.set_target_pane(key, pane_id, cx).is_none()
            || self.connection_focused_pane(key) == Some(pane_id)
        {
            return;
        }
        self.send_layout_to(key, LayoutCommand::FocusPane { pane_id });
    }

    fn set_target_pane(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> Option<bool> {
        if !self
            .connection(key)
            .is_some_and(ServerConnection::can_mutate)
        {
            return None;
        }
        let changed = self.active_connection != key || self.target_pane != Some((key, pane_id));
        self.active_connection = key;
        self.target_pane = Some((key, pane_id));
        if changed {
            cx.notify();
        }
        Some(changed)
    }

    fn send_layout(&mut self, command: LayoutCommand) {
        self.send_layout_to(self.active_connection, command);
    }

    fn send_layout_to(&mut self, key: ConnectionKey, command: LayoutCommand) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let (Some(server_id), Some(session_id)) = (connection.server_id, connection.session_id)
        else {
            return;
        };
        if connection.can_mutate() {
            connection.send(ClientMessage::Layout {
                server_id,
                session_id,
                command,
            });
        }
    }

    fn terminal_command(&mut self, key: ConnectionKey, pane_id: PaneId, command: TerminalCommand) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let (Some(server_id), Some(session_id)) = (connection.server_id, connection.session_id)
        else {
            return;
        };
        if connection.can_mutate()
            && !connection
                .terminals
                .get(&pane_id)
                .is_some_and(|terminal| terminal.exited)
        {
            connection.send(ClientMessage::Terminal {
                server_id,
                session_id,
                pane_id,
                command,
            });
        }
    }

    fn focused_pane(&self) -> Option<PaneId> {
        self.target_pane
            .filter(|(key, _)| *key == self.active_connection)
            .map(|(_, pane_id)| pane_id)
            .or_else(|| {
                Some(
                    self.active_session()?
                        .active_workspace()?
                        .active_tab()
                        .focused_pane()
                        .id(),
                )
            })
    }

    fn rebuild_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection) = self.active_connection() else {
            return;
        };
        let Ok(session) = Session::restore(connection.snapshot.clone()) else {
            return;
        };
        let Some(workspace) = session.active_workspace() else {
            self.target_pane = None;
            return;
        };
        let tab = workspace.active_tab();
        let focused = tab.focused_pane().id();
        let layout = connection
            .zoomed_panes
            .iter()
            .copied()
            .find(|pane_id| tab.panes().iter().any(|pane| pane.id() == *pane_id))
            .map(PaneLayout::Pane)
            .unwrap_or_else(|| tab.layout().clone());
        let key = connection.key;
        self.target_pane = Some((key, focused));
        let dock_layout = self.build_dock_layout(key, &layout, cx);
        self.rebuilding_dock = true;
        self.dock_area.update(cx, |dock, cx| {
            dock.set_locked(true, window, cx);
            dock.set_center(dock_layout, window, cx);
        });
        let owner = cx.weak_entity();
        window.defer(cx, move |_, cx| {
            let _ = owner.update(cx, |this, _| this.rebuilding_dock = false);
        });
    }

    fn build_dock_layout(
        &mut self,
        key: ConnectionKey,
        layout: &PaneLayout,
        cx: &mut Context<Self>,
    ) -> DockLayout {
        match layout {
            PaneLayout::Pane(pane_id) => {
                let panel = self
                    .panels
                    .entry((key, *pane_id))
                    .or_insert_with(|| {
                        let owner = cx.weak_entity();
                        cx.new(|cx| TerminalPanel::new(key, *pane_id, owner, cx))
                    })
                    .clone();
                DockLayout::tabs().panel(panel)
            }
            PaneLayout::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let first = self.build_dock_layout(key, first, cx);
                let second = self.build_dock_layout(key, second, cx);
                let split = match direction {
                    SplitDirection::Horizontal => DockLayout::h_split(),
                    SplitDirection::Vertical => DockLayout::v_split(),
                };
                split
                    .child(first, Some(px(1000. * ratio)))
                    .child(second, Some(px(1000. * (1. - ratio))))
            }
        }
    }

    fn on_dock_layout_changed(&mut self, dock: &Entity<DockArea>, cx: &mut Context<Self>) {
        if self.rebuilding_dock {
            return;
        }
        let Some(session) = self.active_session() else {
            return;
        };
        let Some(tab) = session
            .active_workspace()
            .map(|workspace| workspace.active_tab())
        else {
            return;
        };
        if self
            .active_connection()
            .is_some_and(|connection| !connection.zoomed_panes.is_empty())
        {
            return;
        }
        let mut ratios = Vec::new();
        collect_dock_ratios(&dock.read(cx).dump(cx).center, &mut ratios);
        let mut expected = Vec::new();
        collect_layout_ratios(tab.layout(), &mut expected);
        if ratios.len() == expected.len()
            && ratios
                .iter()
                .zip(expected)
                .any(|(actual, expected)| (actual - expected).abs() > 0.001)
        {
            self.send_layout(LayoutCommand::SetSplitRatios {
                tab_id: tab.id(),
                ratios,
            });
        }
    }

    fn reconnect_active(&mut self) {
        self.start_connect(self.active_connection);
    }

    fn new_workspace(&mut self, root_directory: PathBuf) {
        self.send_layout(LayoutCommand::CreateWorkspace { root_directory });
    }

    fn open_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open Folder".into()),
        });
        let owner = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let path = paths.await.ok()?.ok()??.into_iter().next()?;
                owner.update(cx, |this, _| this.new_workspace(path)).ok()?;
                Some(())
            })
            .detach();
    }

    fn new_tab(&mut self) {
        let Some(workspace_id) = self
            .active_session()
            .and_then(|session| session.active_workspace_id())
        else {
            return;
        };
        self.send_layout(LayoutCommand::CreateTab { workspace_id });
    }

    fn cycle_tab(&mut self, step: isize) {
        let Some(session) = self.active_session() else {
            return;
        };
        let Some(workspace) = session.active_workspace() else {
            return;
        };
        let tabs = workspace.tabs();
        let Some(active_ix) = tabs
            .iter()
            .position(|tab| tab.id() == workspace.active_tab().id())
        else {
            return;
        };
        let target_ix = (active_ix as isize + step).rem_euclid(tabs.len() as isize) as usize;
        self.send_layout(LayoutCommand::ActivateTab {
            tab_id: tabs[target_ix].id(),
        });
    }

    fn move_workspace(&mut self, step: isize) {
        let Some(session) = self.active_session() else {
            return;
        };
        let Some(workspace_id) = session.active_workspace_id() else {
            return;
        };
        let Some(index) = session
            .workspaces()
            .iter()
            .position(|workspace| workspace.id() == workspace_id)
        else {
            return;
        };
        let target = (index as isize + step).clamp(0, session.workspaces().len() as isize - 1);
        self.send_layout(LayoutCommand::MoveWorkspace {
            workspace_id,
            target_index: target as u32,
        });
    }

    fn move_tab(&mut self, step: isize) {
        let Some(session) = self.active_session() else {
            return;
        };
        let Some(workspace) = session.active_workspace() else {
            return;
        };
        let tab_id = workspace.active_tab().id();
        let Some(index) = workspace.tabs().iter().position(|tab| tab.id() == tab_id) else {
            return;
        };
        let target = (index as isize + step).clamp(0, workspace.tabs().len() as isize - 1);
        self.send_layout(LayoutCommand::MoveTab {
            tab_id,
            target_index: target as u32,
        });
    }

    fn split(&mut self, direction: SplitDirection) {
        if let Some(pane_id) = self.focused_pane() {
            self.send_layout(LayoutCommand::SplitPane { pane_id, direction });
        }
    }

    fn focus_direction(&mut self, direction: PaneDirection) {
        if let Some(pane_id) = self.focused_pane() {
            self.send_layout(LayoutCommand::FocusPaneDirection { pane_id, direction });
        }
    }

    fn resize_direction(&mut self, direction: PaneDirection) {
        if let Some(pane_id) = self.focused_pane() {
            self.send_layout(LayoutCommand::ResizePane {
                pane_id,
                direction,
                amount: 0.05,
            });
        }
    }

    fn swap_direction(&mut self, direction: PaneDirection) {
        if let Some(pane_id) = self.focused_pane() {
            self.send_layout(LayoutCommand::SwapPane { pane_id, direction });
        }
    }

    fn toggle_zoom(&mut self) {
        if let Some(pane_id) = self.focused_pane() {
            self.send_layout(LayoutCommand::TogglePaneZoom { pane_id });
        }
    }

    fn close_pane(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pane_id) = self.focused_pane() else {
            return;
        };
        let closes_workspace = self.active_session().is_some_and(|session| {
            session.workspaces().iter().any(|workspace| {
                workspace.tabs().len() == 1
                    && workspace.tabs()[0].panes().len() == 1
                    && workspace.tabs()[0].panes()[0].id() == pane_id
            })
        });
        self.confirm_close(
            LayoutCommand::ClosePane { pane_id },
            closes_workspace,
            window,
            cx,
        );
    }

    fn close_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.active_session() else {
            return;
        };
        let Some(workspace) = session.active_workspace() else {
            return;
        };
        let tab_id = workspace.active_tab().id();
        self.confirm_close(
            LayoutCommand::CloseTab { tab_id },
            workspace.tabs().len() == 1,
            window,
            cx,
        );
    }

    fn close_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(workspace_id) = self
            .active_session()
            .and_then(|session| session.active_workspace_id())
        else {
            return;
        };
        self.confirm_close(
            LayoutCommand::CloseWorkspace { workspace_id },
            true,
            window,
            cx,
        );
    }

    fn confirm_close(
        &mut self,
        command: LayoutCommand,
        closes_workspace: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !closes_workspace {
            self.send_layout(command);
            return;
        }
        let owner = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            window.open_alert_dialog(cx, move |alert, _, _| {
                let owner = owner.clone();
                let command = command.clone();
                alert
                    .confirm()
                    .title("Close Workspace?")
                    .description("The workspace terminals will be stopped. Files are not deleted.")
                    .on_ok(move |_, _, cx| {
                        let _ = owner.update(cx, |this, _| this.send_layout(command.clone()));
                        true
                    })
            });
        });
    }

    fn prompt_text(
        &mut self,
        title: &'static str,
        initial: String,
        apply: impl Fn(&mut Murmur, String) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(initial));
        let owner = cx.weak_entity();
        let apply = Rc::new(apply);
        window.defer(cx, move |window, cx| {
            let input_for_content = input.clone();
            let input_for_ok = input.clone();
            let focus = input.clone();
            let owner = owner.clone();
            let apply = apply.clone();
            window.open_dialog(cx, move |dialog, window, cx| {
                let focus = focus.clone();
                window.defer(cx, move |window, cx| {
                    focus.read(cx).focus_handle(cx).focus(window, cx);
                });
                let input_for_content = input_for_content.clone();
                let input_for_ok = input_for_ok.clone();
                let owner = owner.clone();
                let apply = apply.clone();
                dialog
                    .title(title)
                    .content(move |content, _, _| {
                        content.child(Input::new(&input_for_content).w_full())
                    })
                    .button_props(DialogButtonProps::default().show_cancel(true).on_ok(
                        move |_, _, cx| {
                            let value = input_for_ok.read(cx).value().trim().to_owned();
                            if value.is_empty() {
                                return false;
                            }
                            let apply = apply.clone();
                            let _ = owner.update(cx, |this, _| apply(this, value));
                            true
                        },
                    ))
            });
        });
    }

    fn prompt_add_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_text(
            "Add Server",
            "127.0.0.1:7341".into(),
            |this, value| {
                if let Ok(address) = value.parse::<SocketAddr>() {
                    let endpoint = Endpoint::tcp(address);
                    if this.connections.iter().any(|c| c.endpoint == endpoint) {
                        return;
                    }
                    this.app_error = None;
                    let key = this.next_connection_key;
                    this.next_connection_key += 1;
                    this.connections.push(ServerConnection::new(
                        key,
                        address.to_string(),
                        endpoint,
                    ));
                    this.active_connection = key;
                    this.target_pane = None;
                    this.start_connect(key);
                } else {
                    this.app_error = Some("Invalid server address".into());
                }
            },
            window,
            cx,
        );
    }

    fn prompt_rename_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.active_session() else {
            return;
        };
        let Some(workspace) = session.active_workspace() else {
            return;
        };
        let id = workspace.id();
        let name = workspace.name().to_owned();
        self.prompt_text(
            "Rename Workspace",
            name,
            move |this, name| {
                this.send_layout(LayoutCommand::RenameWorkspace {
                    workspace_id: id,
                    name,
                });
            },
            window,
            cx,
        );
    }

    fn prompt_rename_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.active_session() else {
            return;
        };
        let Some(workspace) = session.active_workspace() else {
            return;
        };
        let tab = workspace.active_tab();
        let id = tab.id();
        let name = tab.name().to_owned();
        self.prompt_text(
            "Rename Tab",
            name,
            move |this, name| {
                this.send_layout(LayoutCommand::RenameTab { tab_id: id, name });
            },
            window,
            cx,
        );
    }

    fn dismiss_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.has_active_dialog(cx) {
            window.close_dialog(cx);
        }
    }

    fn action_add_server(&mut self, _: &AddServer, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.prompt_add_server(window, cx);
    }

    fn action_reconnect(
        &mut self,
        _: &ReconnectServer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.reconnect_active();
        cx.notify();
    }

    fn action_new_workspace(
        &mut self,
        _: &NewWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.new_workspace(home_directory().unwrap_or_else(|| PathBuf::from(".")));
    }

    fn action_open_folder(&mut self, _: &OpenFolder, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.open_folder(window, cx);
    }

    fn action_new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.new_tab();
    }

    fn action_rename_workspace(
        &mut self,
        _: &RenameWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.prompt_rename_workspace(window, cx);
    }

    fn action_rename_tab(&mut self, _: &RenameTab, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.prompt_rename_tab(window, cx);
    }

    fn action_move_workspace_up(
        &mut self,
        _: &MoveWorkspaceUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.move_workspace(-1);
    }

    fn action_move_workspace_down(
        &mut self,
        _: &MoveWorkspaceDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.move_workspace(1);
    }

    fn action_move_tab_left(
        &mut self,
        _: &MoveTabLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.move_tab(-1);
    }

    fn action_move_tab_right(
        &mut self,
        _: &MoveTabRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.move_tab(1);
    }

    fn action_close_pane(&mut self, _: &ClosePane, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.close_pane(window, cx);
    }

    fn action_close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.close_tab(window, cx);
    }

    fn action_close_workspace(
        &mut self,
        _: &CloseWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.close_workspace(window, cx);
    }

    fn action_next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.cycle_tab(1);
    }

    fn action_previous_tab(
        &mut self,
        _: &PreviousTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.cycle_tab(-1);
    }

    fn action_split_right(&mut self, _: &SplitRight, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.split(SplitDirection::Horizontal);
    }

    fn action_split_down(&mut self, _: &SplitDown, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.split(SplitDirection::Vertical);
    }

    fn action_focus_left(&mut self, _: &FocusLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.focus_direction(PaneDirection::Left);
    }

    fn action_focus_right(&mut self, _: &FocusRight, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.focus_direction(PaneDirection::Right);
    }

    fn action_focus_up(&mut self, _: &FocusUp, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.focus_direction(PaneDirection::Up);
    }

    fn action_focus_down(&mut self, _: &FocusDown, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.focus_direction(PaneDirection::Down);
    }

    fn action_resize_left(&mut self, _: &ResizeLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.resize_direction(PaneDirection::Left);
    }

    fn action_resize_right(
        &mut self,
        _: &ResizeRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.resize_direction(PaneDirection::Right);
    }

    fn action_resize_up(&mut self, _: &ResizeUp, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.resize_direction(PaneDirection::Up);
    }

    fn action_resize_down(&mut self, _: &ResizeDown, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.resize_direction(PaneDirection::Down);
    }

    fn action_swap_left(&mut self, _: &SwapLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.swap_direction(PaneDirection::Left);
    }

    fn action_swap_right(&mut self, _: &SwapRight, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.swap_direction(PaneDirection::Right);
    }

    fn action_swap_up(&mut self, _: &SwapUp, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.swap_direction(PaneDirection::Up);
    }

    fn action_swap_down(&mut self, _: &SwapDown, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.swap_direction(PaneDirection::Down);
    }

    fn action_toggle_zoom(&mut self, _: &ToggleZoom, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_dialog(window, cx);
        self.toggle_zoom();
    }

    pub(crate) fn resize_terminal(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        size: TerminalSize,
        cx: &mut Context<Self>,
    ) {
        let pending_key = (key, pane_id);
        if self.pending_sizes.get(&pending_key) == Some(&size)
            || self
                .terminal(key, pane_id)
                .is_some_and(|terminal| terminal.view.size == size)
        {
            return;
        }
        self.pending_sizes.insert(pending_key, size);
        self.terminal_command(key, pane_id, TerminalCommand::Resize(size));
        cx.notify();
    }

    pub(crate) fn update_terminal_geometry(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        bounds: Bounds<Pixels>,
        cell_size: Size<Pixels>,
    ) {
        self.terminal_geometry
            .insert((key, pane_id), TerminalGeometry { bounds, cell_size });
    }

    pub(crate) fn begin_selection(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        position: TerminalPosition,
    ) {
        self.selecting_from = Some((key, pane_id, position));
        self.terminal_command(
            key,
            pane_id,
            TerminalCommand::Select {
                start: position,
                end: position,
            },
        );
    }

    pub(crate) fn update_selection(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        position: TerminalPosition,
    ) {
        if let Some((selected_key, selected_pane, start)) = self.selecting_from
            && selected_key == key
            && selected_pane == pane_id
        {
            self.terminal_command(
                key,
                pane_id,
                TerminalCommand::Select {
                    start,
                    end: position,
                },
            );
        }
    }

    pub(crate) fn is_selecting(&self, key: ConnectionKey, pane_id: PaneId) -> bool {
        self.selecting_from
            .is_some_and(|(selected_key, selected_pane, _)| {
                selected_key == key && selected_pane == pane_id
            })
    }

    pub(crate) fn end_selection(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        position: TerminalPosition,
    ) {
        self.update_selection(key, pane_id, position);
        self.selecting_from = None;
    }

    pub(crate) fn scroll_terminal(&mut self, key: ConnectionKey, pane_id: PaneId, lines: i32) {
        if lines != 0 {
            self.terminal_command(
                key,
                pane_id,
                TerminalCommand::Scroll(TerminalScroll::Lines(lines)),
            );
        }
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some((key, pane_id)) = self.target_pane else {
            return;
        };
        let stroke = &event.keystroke;
        if let Some(action) = fixed_shortcut(stroke) {
            window.dispatch_action(action, cx);
            cx.stop_propagation();
            return;
        }
        let modifiers = stroke.modifiers;
        let copy_paste = modifiers.platform || (modifiers.control && modifiers.shift);
        if copy_paste && stroke.key == "c" {
            self.terminal_command(key, pane_id, TerminalCommand::Copy);
            cx.stop_propagation();
            return;
        }
        if copy_paste && stroke.key == "v" {
            if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                self.terminal_command(key, pane_id, TerminalCommand::Paste(text));
            }
            cx.stop_propagation();
            return;
        }
        if modifiers.shift && stroke.key == "pageup" {
            self.terminal_command(
                key,
                pane_id,
                TerminalCommand::Scroll(TerminalScroll::PageUp),
            );
            cx.stop_propagation();
            return;
        }
        if modifiers.shift && stroke.key == "pagedown" {
            self.terminal_command(
                key,
                pane_id,
                TerminalCommand::Scroll(TerminalScroll::PageDown),
            );
            cx.stop_propagation();
            return;
        }

        let key_code = match stroke.key.as_str() {
            "enter" => Some(TerminalKey::Enter),
            "tab" if modifiers.shift => Some(TerminalKey::BackTab),
            "tab" => Some(TerminalKey::Tab),
            "backspace" => Some(TerminalKey::Backspace),
            "delete" => Some(TerminalKey::Delete),
            "escape" => Some(TerminalKey::Escape),
            "up" => Some(TerminalKey::Up),
            "down" => Some(TerminalKey::Down),
            "right" => Some(TerminalKey::Right),
            "left" => Some(TerminalKey::Left),
            "home" => Some(TerminalKey::Home),
            "end" => Some(TerminalKey::End),
            "pageup" => Some(TerminalKey::PageUp),
            "pagedown" => Some(TerminalKey::PageDown),
            "insert" => Some(TerminalKey::Insert),
            key if key.len() > 1 && key.starts_with('f') => key[1..]
                .parse::<u8>()
                .ok()
                .filter(|number| (1..=12).contains(number))
                .map(TerminalKey::Function),
            _ if modifiers.control || modifiers.alt || modifiers.platform => {
                Some(TerminalKey::Character(
                    stroke
                        .key_char
                        .clone()
                        .unwrap_or_else(|| stroke.key.clone()),
                ))
            }
            _ => None,
        };
        if let Some(key_code) = key_code {
            self.terminal_command(
                key,
                pane_id,
                TerminalCommand::Key {
                    key: key_code,
                    modifiers: TerminalModifiers {
                        shift: modifiers.shift,
                        alt: modifiers.alt,
                        control: modifiers.control,
                        platform: modifiers.platform,
                    },
                },
            );
            cx.stop_propagation();
        }
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let owner = cx.weak_entity();
        let items = self.connections.iter().map(|connection| {
            let key = connection.key;
            let active_server = key == self.active_connection;
            let connected = connection.can_mutate();
            let label = match connection.status {
                ConnectionStatus::Connecting => format!("{} (connecting)", connection.label),
                ConnectionStatus::Connected => connection.label.clone(),
                ConnectionStatus::Disconnected => format!("{} (offline)", connection.label),
            };
            let workspaces = Session::restore(connection.snapshot.clone())
                .ok()
                .map(|session| {
                    let active_workspace = session.active_workspace_id();
                    session
                        .workspaces()
                        .iter()
                        .map(|workspace| {
                            let workspace_id = workspace.id();
                            let owner = owner.clone();
                            SidebarMenuItem::new(workspace.name().to_owned())
                                .icon(IconName::Folder)
                                .active(active_server && active_workspace == Some(workspace_id))
                                .disable(!connected)
                                .on_click(move |_, _, cx| {
                                    let _ = owner.update(cx, |this, _| {
                                        this.select_workspace(key, workspace_id)
                                    });
                                })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            let owner = owner.clone();
            SidebarMenuItem::new(label)
                .icon(IconName::HardDrive)
                .active(active_server)
                .default_open(true)
                .children(workspaces)
                .on_click(move |_, window, cx| {
                    let _ = owner.update(cx, |this, cx| this.select_server(key, window, cx));
                })
        });

        let add_owner = cx.weak_entity();
        let reconnect_owner = cx.weak_entity();
        let reconnect_visible = self
            .active_connection()
            .is_some_and(|connection| connection.status == ConnectionStatus::Disconnected);
        Sidebar::new("murmur-sidebar")
            .collapsible(SidebarCollapsible::None)
            .w_full()
            .header(
                SidebarHeader::new()
                    .child(Icon::new(IconName::SquareTerminal))
                    .child(div().flex_1().font_semibold().child(murmur_core::APP_NAME)),
            )
            .child(SidebarGroup::new("Servers").child(SidebarMenu::new().children(items)))
            .footer(
                SidebarFooter::new()
                    .child(
                        Button::new("add-server")
                            .ghost()
                            .small()
                            .icon(IconName::Plus)
                            .tooltip("Add Server")
                            .on_click(move |_, window, cx| {
                                let _ = add_owner
                                    .update(cx, |this, cx| this.prompt_add_server(window, cx));
                            }),
                    )
                    .when(reconnect_visible, |footer| {
                        footer.child(
                            Button::new("reconnect-server")
                                .ghost()
                                .small()
                                .icon(IconName::LoaderCircle)
                                .tooltip("Reconnect")
                                .on_click(move |_, _, cx| {
                                    let _ = reconnect_owner
                                        .update(cx, |this, _| this.reconnect_active());
                                }),
                        )
                    }),
            )
    }

    fn render_workspace(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(connection) = self.active_connection() else {
            return div().size_full().into_any_element();
        };
        let error = self.app_error.clone().or_else(|| connection.error.clone());
        let can_mutate = connection.can_mutate();
        let Ok(session) = Session::restore(connection.snapshot.clone()) else {
            return div()
                .size_full()
                .child("Invalid Session state")
                .into_any_element();
        };
        let Some(workspace) = session.active_workspace() else {
            let new_owner = cx.weak_entity();
            let folder_owner = cx.weak_entity();
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_3()
                .when_some(error, |view, error| {
                    view.child(div().text_sm().text_color(cx.theme().danger).child(error))
                })
                .child(
                    Button::new("new-terminal-workspace")
                        .primary()
                        .icon(IconName::SquareTerminal)
                        .label("New Terminal Workspace")
                        .disabled(!can_mutate)
                        .on_click(move |_, _, cx| {
                            let _ = new_owner.update(cx, |this, _| {
                                this.new_workspace(
                                    home_directory().unwrap_or_else(|| PathBuf::from(".")),
                                )
                            });
                        }),
                )
                .child(
                    Button::new("open-folder")
                        .outline()
                        .icon(IconName::Folder)
                        .label("Open Folder")
                        .disabled(!can_mutate)
                        .on_click(move |_, window, cx| {
                            let _ =
                                folder_owner.update(cx, |this, cx| this.open_folder(window, cx));
                        }),
                )
                .into_any_element();
        };

        let active_tab = workspace.active_tab().id();
        let tab_buttons = workspace.tabs().iter().map(|tab| {
            let tab_id = tab.id();
            let owner = cx.weak_entity();
            Button::new(("tab", tab_id.as_u64()))
                .ghost()
                .small()
                .selected(tab_id == active_tab)
                .label(tab.name().to_owned())
                .disabled(!can_mutate)
                .on_click(move |_, _, cx| {
                    let _ = owner.update(cx, |this, _| {
                        this.send_layout(LayoutCommand::ActivateTab { tab_id })
                    });
                })
        });
        let new_tab_owner = cx.weak_entity();

        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(px(36.))
                    .flex_shrink_0()
                    .gap_1()
                    .px_2()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .children(tab_buttons)
                    .child(
                        Button::new("new-tab")
                            .ghost()
                            .small()
                            .icon(IconName::Plus)
                            .tooltip("New Tab")
                            .disabled(!can_mutate)
                            .on_click(move |_, _, cx| {
                                let _ = new_tab_owner.update(cx, |this, _| this.new_tab());
                            }),
                    )
                    .when_some(error, |row, error| {
                        row.child(
                            div()
                                .ml_auto()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(cx.theme().danger)
                                .child(error),
                        )
                    }),
            )
            .child(div().min_h_0().flex_1().child(self.dock_area.clone()))
            .into_any_element()
    }
}

impl Focusable for Murmur {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EntityInputHandler for Murmur {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        adjusted_range.replace(
            0..self
                .marked_text
                .as_ref()
                .map_or(0, |text| text.encode_utf16().count()),
        );
        Some(self.marked_text.clone().unwrap_or_default())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: 0..0,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_text
            .as_ref()
            .map(|text| 0..text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked_text = None;
        cx.notify();
    }

    fn paste(&mut self, item: ClipboardItem, _: &mut Window, _: &mut Context<Self>) {
        if let (Some(text), Some((key, pane_id))) = (item.text(), self.target_pane) {
            self.terminal_command(key, pane_id, TerminalCommand::Paste(text));
        }
    }

    fn replace_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        _: &mut Context<Self>,
    ) {
        self.marked_text = None;
        if !text.is_empty()
            && let Some((key, pane_id)) = self.target_pane
        {
            self.terminal_command(key, pane_id, TerminalCommand::Text(text.into()));
        }
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        new_text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked_text = (!new_text.is_empty()).then(|| new_text.to_owned());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let key = self.target_pane?;
        let geometry = self.terminal_geometry.get(&key)?;
        let cursor = self.terminal(key.0, key.1)?.view.cursor?;
        Some(Bounds::new(
            point(
                geometry.bounds.left() + geometry.cell_size.width * f32::from(cursor.column),
                geometry.bounds.top() + geometry.cell_size.height * f32::from(cursor.row),
            ),
            geometry.cell_size,
        ))
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(0)
    }
}

impl Render for Murmur {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("Murmur")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(Self::action_add_server))
            .on_action(cx.listener(Self::action_reconnect))
            .on_action(cx.listener(Self::action_new_workspace))
            .on_action(cx.listener(Self::action_open_folder))
            .on_action(cx.listener(Self::action_new_tab))
            .on_action(cx.listener(Self::action_rename_workspace))
            .on_action(cx.listener(Self::action_rename_tab))
            .on_action(cx.listener(Self::action_move_workspace_up))
            .on_action(cx.listener(Self::action_move_workspace_down))
            .on_action(cx.listener(Self::action_move_tab_left))
            .on_action(cx.listener(Self::action_move_tab_right))
            .on_action(cx.listener(Self::action_close_pane))
            .on_action(cx.listener(Self::action_close_tab))
            .on_action(cx.listener(Self::action_close_workspace))
            .on_action(cx.listener(Self::action_next_tab))
            .on_action(cx.listener(Self::action_previous_tab))
            .on_action(cx.listener(Self::action_split_right))
            .on_action(cx.listener(Self::action_split_down))
            .on_action(cx.listener(Self::action_focus_left))
            .on_action(cx.listener(Self::action_focus_right))
            .on_action(cx.listener(Self::action_focus_up))
            .on_action(cx.listener(Self::action_focus_down))
            .on_action(cx.listener(Self::action_resize_left))
            .on_action(cx.listener(Self::action_resize_right))
            .on_action(cx.listener(Self::action_resize_up))
            .on_action(cx.listener(Self::action_resize_down))
            .on_action(cx.listener(Self::action_swap_left))
            .on_action(cx.listener(Self::action_swap_right))
            .on_action(cx.listener(Self::action_swap_up))
            .on_action(cx.listener(Self::action_swap_down))
            .on_action(cx.listener(Self::action_toggle_zoom))
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                h_resizable("murmur-shell")
                    .child(
                        resizable_panel()
                            .size(px(240.))
                            .size_range(px(180.)..px(360.))
                            .flex_none()
                            .child(self.render_sidebar(cx)),
                    )
                    .child(self.render_workspace(cx)),
            )
    }
}

fn collect_layout_ratios(layout: &PaneLayout, ratios: &mut Vec<f32>) {
    if let PaneLayout::Split {
        ratio,
        first,
        second,
        ..
    } = layout
    {
        ratios.push(*ratio);
        collect_layout_ratios(first, ratios);
        collect_layout_ratios(second, ratios);
    }
}

fn collect_dock_ratios(state: &PanelState, ratios: &mut Vec<f32>) {
    if let PanelInfo::Stack { sizes, .. } = &state.info
        && sizes.len() == 2
    {
        let total = sizes[0] + sizes[1];
        if total > px(0.) {
            ratios.push((sizes[0] / total).clamp(0.1, 0.9));
        }
    }
    for child in &state.children {
        collect_dock_ratios(child, ratios);
    }
}

fn home_directory() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

fn fixed_shortcut(stroke: &Keystroke) -> Option<Box<dyn Action>> {
    let modifiers = stroke.modifiers;
    if modifiers.platform || modifiers.function {
        return None;
    }
    match (
        modifiers.control,
        modifiers.alt,
        modifiers.shift,
        stroke.key.as_str(),
    ) {
        (true, false, true, "t") => Some(Box::new(NewTab)),
        (true, false, true, "w") => Some(Box::new(ClosePane)),
        (true, false, false, "tab") => Some(Box::new(NextTab)),
        (true, false, true, "tab") => Some(Box::new(PreviousTab)),
        (false, true, true, "=") => Some(Box::new(SplitRight)),
        (false, true, true, "-") => Some(Box::new(SplitDown)),
        (false, true, false, "left") => Some(Box::new(FocusLeft)),
        (false, true, false, "right") => Some(Box::new(FocusRight)),
        (false, true, false, "up") => Some(Box::new(FocusUp)),
        (false, true, false, "down") => Some(Box::new(FocusDown)),
        (false, true, true, "left") => Some(Box::new(ResizeLeft)),
        (false, true, true, "right") => Some(Box::new(ResizeRight)),
        (false, true, true, "up") => Some(Box::new(ResizeUp)),
        (false, true, true, "down") => Some(Box::new(ResizeDown)),
        _ => None,
    }
}

fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("ctrl-shift-t", NewTab, Some("Murmur")),
        KeyBinding::new("ctrl-shift-w", ClosePane, Some("Murmur")),
        KeyBinding::new("ctrl-tab", NextTab, Some("Murmur")),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, Some("Murmur")),
        KeyBinding::new("alt-shift-=", SplitRight, Some("Murmur")),
        KeyBinding::new("alt-shift--", SplitDown, Some("Murmur")),
        KeyBinding::new("alt-left", FocusLeft, Some("Murmur")),
        KeyBinding::new("alt-right", FocusRight, Some("Murmur")),
        KeyBinding::new("alt-up", FocusUp, Some("Murmur")),
        KeyBinding::new("alt-down", FocusDown, Some("Murmur")),
        KeyBinding::new("alt-shift-left", ResizeLeft, Some("Murmur")),
        KeyBinding::new("alt-shift-right", ResizeRight, Some("Murmur")),
        KeyBinding::new("alt-shift-up", ResizeUp, Some("Murmur")),
        KeyBinding::new("alt-shift-down", ResizeDown, Some("Murmur")),
    ]);
}

fn main() {
    let endpoint =
        murmur_server::ensure_local_server().unwrap_or_else(|_| ServerConfig::default().endpoint);
    let initial =
        ClientConnection::connect(&endpoint, "murmur-gui").map_err(|error| error.to_string());
    let app = gpui_platform::application().with_assets(Assets);

    app.run(move |cx| {
        gpui_component::init(cx);
        bind_keys(cx);
        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1280.0), px(720.0)), cx)),
            ..Default::default()
        };
        cx.spawn(async move |cx| {
            cx.open_window(window_options, |window, cx| {
                let view = cx.new(|cx| Murmur::new(endpoint, initial, window, cx));
                let root = cx.new(|cx| Root::new(view, window, cx));
                window.resize(size(px(1280.0), px(720.0)));
                root
            })
            .expect("failed to open Murmur window");
        })
        .detach();
    });
}

#[cfg(test)]
mod tests {
    use gpui::Keystroke;

    use super::{FocusLeft, NextTab, PreviousTab, SplitRight, fixed_shortcut};

    #[test]
    fn terminal_shortcut_fallback_maps_only_fixed_chords() {
        let action = |keys: &str| fixed_shortcut(&Keystroke::parse(keys).unwrap());

        assert!(action("ctrl-tab").unwrap().as_any().is::<NextTab>());
        assert!(
            action("ctrl-shift-tab")
                .unwrap()
                .as_any()
                .is::<PreviousTab>()
        );
        assert!(action("alt-shift-=").unwrap().as_any().is::<SplitRight>());
        assert!(action("alt-left").unwrap().as_any().is::<FocusLeft>());
        assert!(action("ctrl-p").is_none());
    }
}
