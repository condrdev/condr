//! The GUI's own window state (ADR 0023): what this Client showed when it last ran, so the
//! next run opens the same way. Everything here is Client presentation; the Server never
//! sees it and `config.toml` never carries it. Stored as one protobuf message (ADR 0028), in
//! the platform state directory, rewritten whole after every change with a debounce.

use super::*;
use prost::Message as _;
use std::fs;
use std::path::Path;

pub(super) const STATE_FILE_NAME: &str = "condr-gui.state";
pub(super) const STATE_SAVE_DEBOUNCE: Duration = Duration::from_millis(500);
/// Far above any real state; a file past it is someone else's.
const MAX_STATE_BYTES: usize = 1024 * 1024;

/// What startup read: where the file is, and what it said. A `None` path never writes.
#[derive(Clone, Debug, Default)]
pub(super) struct LoadedState {
    pub path: Option<PathBuf>,
    pub state: GuiState,
}

impl LoadedState {
    pub(super) fn read(path: Option<PathBuf>) -> Self {
        let state = GuiState::read(path.as_deref());
        Self { path, state }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct GuiState {
    pub window: Option<WindowState>,
    /// Logical pixels, as the handles set them; `None` keeps the defaults.
    pub sidebar_width: Option<f32>,
    pub sidebar_collapsed: bool,
    pub changes_width: Option<f32>,
    /// The Device whose Workspace the window showed.
    pub active_server: Option<ServerId>,
    /// Keyed by the Server's own id, which outlives connection order, Device renames
    /// and Server restarts; a Server never seen again keeps its entry.
    pub servers: HashMap<ServerId, ServerState>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct ServerState {
    pub view_workspace: Option<WorkspaceId>,
    pub workspaces: HashMap<WorkspaceId, WorkspaceState>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct WorkspaceState {
    pub view_tab: Option<TabId>,
    /// Unfolded in the left sidebar.
    pub sidebar_open: bool,
    /// The right sidebar is open, and which view it shows when the user chose one.
    pub changes_open: bool,
    pub sidebar_view: Option<SidebarView>,
}

/// GPUI's `WindowBounds` with plain numbers, since it has no serde of its own.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum WindowState {
    Windowed(WindowRect),
    /// The rectangle is the restore size, as GPUI keeps it.
    Maximized(WindowRect),
    Fullscreen(WindowRect),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct WindowRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl From<Bounds<Pixels>> for WindowRect {
    fn from(bounds: Bounds<Pixels>) -> Self {
        Self {
            x: bounds.origin.x.into(),
            y: bounds.origin.y.into(),
            width: bounds.size.width.into(),
            height: bounds.size.height.into(),
        }
    }
}

impl From<WindowRect> for Bounds<Pixels> {
    fn from(rect: WindowRect) -> Self {
        Bounds::new(
            point(px(rect.x), px(rect.y)),
            size(px(rect.width), px(rect.height)),
        )
    }
}

impl From<WindowBounds> for WindowState {
    fn from(bounds: WindowBounds) -> Self {
        match bounds {
            WindowBounds::Windowed(bounds) => Self::Windowed(bounds.into()),
            WindowBounds::Maximized(bounds) => Self::Maximized(bounds.into()),
            WindowBounds::Fullscreen(bounds) => Self::Fullscreen(bounds.into()),
        }
    }
}

impl WindowState {
    fn rect(self) -> WindowRect {
        match self {
            Self::Windowed(rect) | Self::Maximized(rect) | Self::Fullscreen(rect) => rect,
        }
    }

    /// The bounds to open with: the saved ones when their centre is still on a display,
    /// else the default centred window (a monitor was unplugged, or the state came from
    /// another machine). A maximized or fullscreen window stays so either way.
    pub(super) fn window_bounds(self, default_size: Size<Pixels>, cx: &App) -> WindowBounds {
        let displays: Vec<Bounds<Pixels>> = cx
            .displays()
            .iter()
            .map(|display| display.bounds())
            .collect();
        let rect: Bounds<Pixels> = if on_a_display(self.rect(), &displays) {
            self.rect().into()
        } else {
            match WindowBounds::centered(default_size, cx) {
                WindowBounds::Windowed(bounds)
                | WindowBounds::Maximized(bounds)
                | WindowBounds::Fullscreen(bounds) => bounds,
            }
        };
        match self {
            Self::Windowed(_) => WindowBounds::Windowed(rect),
            Self::Maximized(_) => WindowBounds::Maximized(rect),
            Self::Fullscreen(_) => WindowBounds::Fullscreen(rect),
        }
    }
}

/// Whether a window at `rect` would have its centre on one of `displays`; a rect with no
/// size counts as off-screen. No displays at all (a headless test) keeps the rect.
fn on_a_display(rect: WindowRect, displays: &[Bounds<Pixels>]) -> bool {
    if rect.width <= 0. || rect.height <= 0. {
        return false;
    }
    if displays.is_empty() {
        return true;
    }
    let centre = point(px(rect.x + rect.width / 2.), px(rect.y + rect.height / 2.));
    displays.iter().any(|display| display.contains(&centre))
}

impl GuiState {
    pub(super) fn default_path() -> Option<PathBuf> {
        condr_core::state_directory().map(|directory| directory.join(STATE_FILE_NAME))
    }

    /// A missing or unreadable file is an empty state: this is a convenience, never a
    /// reason to refuse to start.
    pub(super) fn read(path: Option<&Path>) -> Self {
        let Some(path) = path else {
            return Self::default();
        };
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Self::default();
            }
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "GUI state unreadable; starting fresh");
                return Self::default();
            }
        };
        match Self::from_bytes(&bytes) {
            Ok(state) => state,
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "GUI state undecodable; starting fresh");
                Self::default()
            }
        }
    }

    /// Writes the whole state through a sibling temporary file and a rename, so a crash
    /// mid-write leaves the previous state, not a torn one.
    pub(super) fn write(&self, path: &Path) -> std::io::Result<()> {
        let bytes = self
            .to_bytes()
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("state.tmp");
        fs::write(&temporary, bytes)?;
        fs::rename(&temporary, path)
    }

    fn to_bytes(&self) -> Result<Vec<u8>, String> {
        let wire = wire::GuiState::from(self);
        if wire.encoded_len() > MAX_STATE_BYTES {
            return Err(format!("GUI state exceeds {MAX_STATE_BYTES} bytes"));
        }
        Ok(wire.encode_to_vec())
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        if bytes.len() > MAX_STATE_BYTES {
            return Err(format!("GUI state exceeds {MAX_STATE_BYTES} bytes"));
        }
        wire::GuiState::decode(bytes)
            .map(Self::from)
            .map_err(|error| error.to_string())
    }
}

/// The file's protobuf shape. Private to the GUI, so it is declared here rather than in
/// `proto/`: nothing else reads it. A value this build does not know is dropped, which
/// costs a window position or a sidebar choice at most.
mod wire {
    use super::{SidebarView, WindowRect, WindowState};
    use condr_core::protocol::ServerId;
    use condr_core::{TabId, WorkspaceId};
    use std::collections::HashMap;

    #[derive(Clone, PartialEq, prost::Message)]
    pub(super) struct GuiState {
        #[prost(message, optional, tag = "1")]
        window: Option<Window>,
        #[prost(float, optional, tag = "2")]
        sidebar_width: Option<f32>,
        #[prost(bool, tag = "3")]
        sidebar_collapsed: bool,
        #[prost(float, optional, tag = "4")]
        changes_width: Option<f32>,
        #[prost(uint64, optional, tag = "5")]
        active_server: Option<u64>,
        #[prost(map = "uint64, message", tag = "6")]
        servers: HashMap<u64, Server>,
    }

    /// `mode` 1 windowed, 2 maximized, 3 fullscreen.
    #[derive(Clone, PartialEq, prost::Message)]
    struct Window {
        #[prost(uint32, tag = "1")]
        mode: u32,
        #[prost(float, tag = "2")]
        x: f32,
        #[prost(float, tag = "3")]
        y: f32,
        #[prost(float, tag = "4")]
        width: f32,
        #[prost(float, tag = "5")]
        height: f32,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    struct Server {
        #[prost(uint64, optional, tag = "1")]
        view_workspace: Option<u64>,
        #[prost(map = "uint64, message", tag = "2")]
        workspaces: HashMap<u64, Workspace>,
    }

    /// `sidebar_view` 1 Changes, 2 Files.
    #[derive(Clone, PartialEq, prost::Message)]
    struct Workspace {
        #[prost(uint64, optional, tag = "1")]
        view_tab: Option<u64>,
        #[prost(bool, tag = "2")]
        sidebar_open: bool,
        #[prost(bool, tag = "3")]
        changes_open: bool,
        #[prost(uint32, optional, tag = "4")]
        sidebar_view: Option<u32>,
    }

    impl From<&super::GuiState> for GuiState {
        fn from(state: &super::GuiState) -> Self {
            Self {
                window: state.window.map(|window| {
                    let (mode, rect) = match window {
                        WindowState::Windowed(rect) => (1, rect),
                        WindowState::Maximized(rect) => (2, rect),
                        WindowState::Fullscreen(rect) => (3, rect),
                    };
                    Window {
                        mode,
                        x: rect.x,
                        y: rect.y,
                        width: rect.width,
                        height: rect.height,
                    }
                }),
                sidebar_width: state.sidebar_width,
                sidebar_collapsed: state.sidebar_collapsed,
                changes_width: state.changes_width,
                active_server: state.active_server.map(|server| server.0),
                servers: state
                    .servers
                    .iter()
                    .map(|(server, state)| {
                        let workspaces = state
                            .workspaces
                            .iter()
                            .map(|(workspace, state)| {
                                let workspace_state = Workspace {
                                    view_tab: state.view_tab.map(TabId::as_u64),
                                    sidebar_open: state.sidebar_open,
                                    changes_open: state.changes_open,
                                    sidebar_view: state.sidebar_view.map(|view| match view {
                                        SidebarView::Changes => 1,
                                        SidebarView::Files => 2,
                                    }),
                                };
                                (workspace.as_u64(), workspace_state)
                            })
                            .collect();
                        let server_state = Server {
                            view_workspace: state.view_workspace.map(WorkspaceId::as_u64),
                            workspaces,
                        };
                        (server.0, server_state)
                    })
                    .collect(),
            }
        }
    }

    impl From<GuiState> for super::GuiState {
        fn from(state: GuiState) -> Self {
            Self {
                window: state.window.and_then(|window| {
                    let rect = WindowRect {
                        x: window.x,
                        y: window.y,
                        width: window.width,
                        height: window.height,
                    };
                    match window.mode {
                        1 => Some(WindowState::Windowed(rect)),
                        2 => Some(WindowState::Maximized(rect)),
                        3 => Some(WindowState::Fullscreen(rect)),
                        _ => None,
                    }
                }),
                sidebar_width: state.sidebar_width,
                sidebar_collapsed: state.sidebar_collapsed,
                changes_width: state.changes_width,
                active_server: state.active_server.map(ServerId),
                servers: state
                    .servers
                    .into_iter()
                    .map(|(server, state)| {
                        let workspaces = state
                            .workspaces
                            .into_iter()
                            .map(|(workspace, state)| {
                                let workspace_state = super::WorkspaceState {
                                    view_tab: state.view_tab.map(TabId::from_u64),
                                    sidebar_open: state.sidebar_open,
                                    changes_open: state.changes_open,
                                    sidebar_view: state.sidebar_view.and_then(|view| match view {
                                        1 => Some(SidebarView::Changes),
                                        2 => Some(SidebarView::Files),
                                        _ => None,
                                    }),
                                };
                                (WorkspaceId::from_u64(workspace), workspace_state)
                            })
                            .collect();
                        let server_state = super::ServerState {
                            view_workspace: state.view_workspace.map(WorkspaceId::from_u64),
                            workspaces,
                        };
                        (ServerId(server), server_state)
                    })
                    .collect(),
            }
        }
    }
}

impl Condr {
    /// Something the state file records changed: rewrite it after a quiet half second.
    /// Cheap to call from every presentation change, so callers need not be selective.
    pub(super) fn schedule_state_save(&mut self, cx: &mut Context<Self>) {
        if self.state_path.is_none() {
            return;
        }
        self.state_save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(STATE_SAVE_DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| this.save_state_now(cx));
        }));
    }

    /// Writes the current state without waiting; the write itself runs off the main
    /// thread and is awaited on quit.
    pub(super) fn save_state_now(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.state_path.clone() else {
            return;
        };
        let state = self.collect_state(cx);
        self.state_save = Some(cx.background_spawn(async move {
            if let Err(error) = state.write(&path) {
                tracing::warn!(path = %path.display(), %error, "failed to save GUI state");
            }
        }));
    }

    /// The state as it stands: connected Servers are written from their live view, every
    /// other Server keeps what the file had.
    pub(super) fn collect_state(&self, cx: &App) -> GuiState {
        let mut servers = self.restored_state.servers.clone();
        for connection in &self.connections {
            let (Some(server_id), Some(session)) = (connection.server_id, connection.session())
            else {
                continue;
            };
            let key = connection.key;
            let workspaces = session
                .workspaces()
                .iter()
                .map(|workspace| {
                    let workspace_id = workspace.id();
                    let state = WorkspaceState {
                        view_tab: connection.view_tabs.get(&workspace_id).copied(),
                        sidebar_open: self
                            .sidebar_workspace_open
                            .get(&(key, workspace_id))
                            .is_some_and(|open| *open.read(cx)),
                        changes_open: self.changes_open.contains(&(key, workspace_id)),
                        sidebar_view: self.sidebar_views.get(&(key, workspace_id)).copied(),
                    };
                    (workspace_id, state)
                })
                .collect();
            servers.insert(
                server_id,
                ServerState {
                    view_workspace: connection.view_workspace,
                    workspaces,
                },
            );
        }
        GuiState {
            window: self.window_state,
            sidebar_width: Some(self.sidebar_width.into()),
            sidebar_collapsed: self.sidebar_collapsed,
            changes_width: Some(self.changes_width.into()),
            // Until the active connection has bootstrapped (a slow first start), the
            // file's answer stands rather than `None`.
            active_server: self
                .active_connection()
                .and_then(|connection| connection.server_id)
                .or(self.restored_state.active_server),
            servers,
        }
    }

    /// A Server's first Bootstrap on this run: put its Workspaces back the way the file
    /// remembers them. Ids the Session no longer has are skipped; `set_view` and the
    /// sidebar sync already run before this, so entries exist for every live Workspace.
    pub(super) fn restore_server_state(&mut self, key: ConnectionKey, cx: &mut Context<Self>) {
        let Some(connection) = self.connection(key) else {
            return;
        };
        let (Some(server_id), Some(session)) = (connection.server_id, connection.session()) else {
            return;
        };
        let Some(saved) = self.restored_state.servers.get(&server_id).cloned() else {
            return;
        };
        for workspace in session.workspaces() {
            let workspace_id = workspace.id();
            let Some(state) = saved.workspaces.get(&workspace_id) else {
                continue;
            };
            if let Some(open) = self.sidebar_workspace_open.get(&(key, workspace_id)) {
                open.update(cx, |open, cx| {
                    if *open != state.sidebar_open {
                        *open = state.sidebar_open;
                        cx.notify();
                    }
                });
            }
            if state.changes_open {
                self.changes_open.insert((key, workspace_id));
            } else {
                self.changes_open.remove(&(key, workspace_id));
            }
            if let Some(view) = state.sidebar_view {
                self.sidebar_views.insert((key, workspace_id), view);
            }
            if let Some(tab_id) = state.view_tab
                && workspace.tab(tab_id).is_some()
                && let Some(connection) = self.connection_mut(key)
            {
                connection.view_tabs.insert(workspace_id, tab_id);
            }
        }
        if let Some(workspace_id) = saved.view_workspace
            && let Some(connection) = self.connection_mut(key)
        {
            connection.set_view(workspace_id, None);
        }
        if self.restored_state.active_server == Some(server_id) {
            self.active_connection = key;
        }
        self.refresh_target_pane(key);
    }
}

#[cfg(test)]
mod tests {
    // Not `super::*`: the app module's glob brings in GPUI's `#[test]` macro.
    use super::{
        GuiState, STATE_FILE_NAME, ServerState, WindowRect, WindowState, WorkspaceState,
        on_a_display,
    };
    use crate::app::changes::SidebarView;
    use condr_core::protocol::ServerId;
    use condr_core::{TabId, WorkspaceId};
    use gpui_kit::{Bounds, point, px, size};
    use std::collections::HashMap;
    use std::fs;

    #[test]
    fn the_state_survives_a_round_trip_and_an_empty_file_is_the_default() {
        let mut state = GuiState {
            window: Some(WindowState::Maximized(WindowRect {
                x: 10.,
                y: 20.,
                width: 1280.,
                height: 720.,
            })),
            sidebar_width: Some(240.),
            sidebar_collapsed: true,
            changes_width: Some(300.),
            active_server: Some(ServerId(7)),
            servers: HashMap::new(),
        };
        let mut workspaces = HashMap::new();
        workspaces.insert(
            WorkspaceId::from_u64(3),
            WorkspaceState {
                view_tab: Some(TabId::from_u64(9)),
                sidebar_open: true,
                changes_open: true,
                sidebar_view: Some(SidebarView::Files),
            },
        );
        state.servers.insert(
            ServerId(7),
            ServerState {
                view_workspace: Some(WorkspaceId::from_u64(3)),
                workspaces,
            },
        );
        let bytes = state.to_bytes().unwrap();
        assert_eq!(GuiState::from_bytes(&bytes).unwrap(), state);
        assert_eq!(GuiState::read(None), GuiState::default());
        assert!(GuiState::from_bytes(b"not a state").is_err());

        let directory = std::env::temp_dir().join(format!(
            "condr-gui-state-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&directory);
        let path = directory.join("nested").join(STATE_FILE_NAME);
        assert_eq!(GuiState::read(Some(&path)), GuiState::default());
        state.write(&path).unwrap();
        assert_eq!(GuiState::read(Some(&path)), state);
        assert!(!path.with_extension("state.tmp").exists());
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn a_window_off_every_display_falls_back() {
        let displays = [
            Bounds::new(point(px(0.), px(0.)), size(px(1920.), px(1080.))),
            Bounds::new(point(px(1920.), px(0.)), size(px(2560.), px(1440.))),
        ];
        let on_second = WindowRect {
            x: 2000.,
            y: 100.,
            width: 1280.,
            height: 720.,
        };
        let unplugged = WindowRect {
            x: -3000.,
            y: 100.,
            width: 1280.,
            height: 720.,
        };
        let hanging_off = WindowRect {
            x: 1600.,
            y: 900.,
            width: 1280.,
            height: 720.,
        };
        let empty = WindowRect {
            x: 10.,
            y: 10.,
            width: 0.,
            height: 0.,
        };
        assert!(on_a_display(on_second, &displays));
        assert!(!on_a_display(unplugged, &displays));
        // Its centre is on the second display even though its corner is on the first.
        assert!(on_a_display(hanging_off, &displays));
        assert!(!on_a_display(empty, &displays));
        assert!(on_a_display(on_second, &[]));
    }
}
