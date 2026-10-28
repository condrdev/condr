use super::*;

/// What a window shows of the Session tree (ADR 0043). Detaching moves: a Device,
/// Workspace or Tab is drawn by exactly one window, the one with the most specific
/// scope holding it, and the place it left shows a page instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum WindowScope {
    /// Everything: the main window.
    Main,
    Device(ConnectionKey),
    Workspace(ConnectionKey, WorkspaceId),
    Tab(ConnectionKey, TabId),
}

impl WindowScope {
    fn depth(self) -> u8 {
        match self {
            Self::Main => 0,
            Self::Device(_) => 1,
            Self::Workspace(..) => 2,
            Self::Tab(..) => 3,
        }
    }

    /// Whether the left sidebar of a window with this scope lists a Device's rows.
    pub(super) fn lists_connection(self, key: ConnectionKey) -> bool {
        self.connection().is_none_or(|scope_key| scope_key == key)
    }

    /// Whether it lists a Workspace's rows, "Needs you" included.
    pub(super) fn lists_workspace(self, key: ConnectionKey, workspace_id: WorkspaceId) -> bool {
        match self {
            Self::Main => true,
            Self::Device(scope_key) => scope_key == key,
            Self::Workspace(scope_key, scope_workspace) => {
                scope_key == key && scope_workspace == workspace_id
            }
            Self::Tab(..) => false,
        }
    }

    pub(super) fn connection(self) -> Option<ConnectionKey> {
        match self {
            Self::Main => None,
            Self::Device(key) | Self::Workspace(key, _) | Self::Tab(key, _) => Some(key),
        }
    }
}

/// What one of this Client's windows keeps for itself; the connections, the Panes, the
/// Dock surfaces and every Workspace's view are the application's, so they follow
/// whatever moves between windows. `Condr` is the root view of each window and finds
/// its own by the window it draws in.
pub(super) struct WindowView {
    pub(super) handle: AnyWindowHandle,
    pub(super) scope: WindowScope,
    pub(super) focus_handle: FocusHandle,
    /// The Dock surface this window draws, once `rebuild_window_dock` has prepared it.
    pub(super) dock_surface: Option<DockSurfaceKey>,
    /// The content area between the sidebars, as last laid out.
    pub(super) workspace_size: Size<Pixels>,
    /// Absolute, as Zed keeps its dock sizes: a window resize never changes it,
    /// only dragging the handle does.
    pub(super) sidebar_width: Pixels,
    pub(super) sidebar_collapsed: bool,
    pub(super) changes_width: Pixels,
    /// The title bar's Tab strip scrolls when the Tabs outgrow it; a newly active Tab
    /// is scrolled into view once, which `last_scrolled_tab` remembers across renders.
    pub(super) tab_strip_scroll: ScrollHandle,
    pub(super) last_scrolled_tab: Cell<Option<TabId>>,
    /// Where the window is, as last observed, so a save needs no `Window` (ADR 0023).
    pub(super) bounds: gui_state::WindowState,
    pub(super) display: Option<DisplayId>,
    _activation_subscription: Subscription,
    _bounds_subscription: Subscription,
}

impl WindowView {
    pub(super) fn new(window: &mut Window, scope: WindowScope, cx: &mut Context<Condr>) -> Self {
        let activation_subscription = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() && this.owns_window(window) {
                // The keyboard follows the window last activated, and so does the focus
                // this Client reports.
                this.last_active = window.window_handle();
                this.rebuild_window_dock(window, cx);
            }
            this.end_pointer_gesture_on_deactivation(window);
            this.sync_terminal_focus(window, cx);
        });
        let bounds_subscription = cx.observe_window_bounds(window, |this, window, cx| {
            let display = window.display(cx).map(|display| display.id());
            if this.owns_window(window) {
                let view = this.window_view_mut(window);
                view.bounds = window.window_bounds().into();
                view.display = display;
                this.schedule_state_save(cx);
            }
        });
        Self {
            handle: window.window_handle(),
            scope,
            focus_handle: cx.focus_handle(),
            dock_surface: None,
            workspace_size: size(
                (window.viewport_size().width - INITIAL_SIDEBAR_WIDTH).max(px(0.)),
                (window.viewport_size().height - WORKSPACE_TITLE_BAR_HEIGHT).max(px(0.)),
            ),
            sidebar_width: INITIAL_SIDEBAR_WIDTH,
            sidebar_collapsed: false,
            changes_width: INITIAL_CHANGES_WIDTH,
            tab_strip_scroll: ScrollHandle::new(),
            last_scrolled_tab: Cell::new(None),
            bounds: window.window_bounds().into(),
            display: window.display(cx).map(|display| display.id()),
            _activation_subscription: activation_subscription,
            _bounds_subscription: bounds_subscription,
        }
    }

    pub(super) fn sidebar_layout_width(&self) -> Pixels {
        if self.sidebar_collapsed {
            COLLAPSED_SIDEBAR_WIDTH
        } else {
            self.sidebar_width
        }
    }
}

impl Condr {
    /// Remembers what a Tab, Workspace or Device drag would detach: GPUI hands the dragged
    /// value only to moves, and the release may come outside the window (ADR 0043).
    pub(super) fn track_drag_out(&mut self, scope: WindowScope) {
        self.drag_out = Some(scope);
    }

    /// A drag released outside its window opens the unit there, at the cursor. A Pane's
    /// drag is the Dock's, and stays a cancelled drag.
    pub(super) fn release_outside(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // An occluding element under the cursor also counts as "outside" the root; only
        // the window's own edge does here. A maximized window has no outside.
        let inside = Bounds::new(Point::default(), window.viewport_size()).contains(&position);
        if let Some(scope) = self.drag_out.take()
            && !inside
        {
            self.open_in_new_window_at(scope, position, window, cx);
        }
    }

    pub(super) fn owns_window(&self, window: &Window) -> bool {
        let handle = window.window_handle();
        self.windows.iter().any(|view| view.handle == handle)
    }

    pub(super) fn window_view(&self, window: &Window) -> &WindowView {
        let handle = window.window_handle();
        self.windows
            .iter()
            .find(|view| view.handle == handle)
            .expect("Condr draws only in its own windows")
    }

    pub(super) fn window_view_mut(&mut self, window: &Window) -> &mut WindowView {
        let handle = window.window_handle();
        self.windows
            .iter_mut()
            .find(|view| view.handle == handle)
            .expect("Condr draws only in its own windows")
    }

    /// The main window: the application's own, which quits it when closed, and where a
    /// failure with no window behind it is shown (ADR 0043).
    pub(super) fn main_view(&self) -> &WindowView {
        &self.windows[0]
    }

    pub(super) fn main_window(&self) -> AnyWindowHandle {
        self.main_view().handle
    }

    pub(super) fn is_main(&self, window: &Window) -> bool {
        window.window_handle() == self.main_window()
    }

    /// The window last activated: the one the keyboard, the actions and the reported
    /// focus belong to.
    pub(super) fn active_view(&self) -> &WindowView {
        self.windows
            .iter()
            .find(|view| view.handle == self.last_active)
            .unwrap_or_else(|| self.main_view())
    }

    /// The connection a window presents: the main window's choice, else its scope's.
    pub(super) fn presented_key(&self, view: &WindowView) -> ConnectionKey {
        view.scope.connection().unwrap_or(self.active_connection)
    }

    /// The Workspace and Tab a window presents, whether it draws them or shows the page
    /// saying another window does.
    pub(super) fn presented_in(
        &self,
        view: &WindowView,
    ) -> Option<(ConnectionKey, &Session, WorkspaceId, TabId)> {
        let key = self.presented_key(view);
        let connection = self.connection(key)?;
        let session = connection.presented_session()?;
        let (workspace_id, tab_id) = match view.scope {
            WindowScope::Main | WindowScope::Device(_) => connection.viewed(session)?,
            WindowScope::Workspace(_, workspace_id) => (
                workspace_id,
                connection.viewed_tab_id(session, workspace_id)?,
            ),
            WindowScope::Tab(_, tab_id) => (workspace_of_tab(session, tab_id)?, tab_id),
        };
        Some((key, session, workspace_id, tab_id))
    }

    /// The window that draws a unit: the one whose scope is the most specific holding it.
    pub(super) fn home_window(
        &self,
        key: ConnectionKey,
        workspace_id: Option<WorkspaceId>,
        tab_id: Option<TabId>,
    ) -> AnyWindowHandle {
        [
            tab_id.map(|tab_id| WindowScope::Tab(key, tab_id)),
            workspace_id.map(|workspace_id| WindowScope::Workspace(key, workspace_id)),
            Some(WindowScope::Device(key)),
        ]
        .into_iter()
        .flatten()
        .find_map(|scope| self.window_for(scope))
        .unwrap_or_else(|| self.main_window())
    }

    pub(super) fn window_for(&self, scope: WindowScope) -> Option<AnyWindowHandle> {
        self.windows
            .iter()
            .find(|view| view.scope == scope)
            .map(|view| view.handle)
    }

    /// What this window presents but another window draws: the outermost of its Device,
    /// Workspace and Tab that has a window of its own, with that window.
    pub(super) fn shown_elsewhere(
        &self,
        view: &WindowView,
    ) -> Option<(WindowScope, AnyWindowHandle)> {
        let key = self.presented_key(view);
        let presented = self.presented_in(view);
        [
            Some(WindowScope::Device(key)),
            presented.map(|(_, _, workspace_id, _)| WindowScope::Workspace(key, workspace_id)),
            presented.map(|(_, _, _, tab_id)| WindowScope::Tab(key, tab_id)),
        ]
        .into_iter()
        .flatten()
        .filter(|scope| scope.depth() > view.scope.depth())
        .find_map(|scope| {
            self.window_for(scope)
                .filter(|handle| *handle != view.handle)
                .map(|handle| (scope, handle))
        })
    }

    /// Whether this window draws the Workspace it presents, and so its right sidebar: a
    /// Tab shown elsewhere leaves the Workspace here (ADR 0043).
    pub(super) fn draws_workspace(&self, view: &WindowView) -> bool {
        !matches!(
            self.shown_elsewhere(view),
            Some((WindowScope::Device(_) | WindowScope::Workspace(..), _))
        )
    }

    /// What each detached window shows of connection `key`: its Tab's Dock projection or
    /// viewer file. A change there needs a rebuild that the connection, which compares
    /// only the Tab it shows itself, would not ask for.
    pub(super) fn detached_presentations(
        &self,
        key: ConnectionKey,
    ) -> Vec<(TabId, Option<PaneLayout>, Option<RelativePathBuf>)> {
        let Some(connection) = self.connection(key) else {
            return Vec::new();
        };
        self.windows[1..]
            .iter()
            .filter(|view| self.presented_key(view) == key)
            .filter_map(|view| {
                let (_, session, _, tab_id) = self.presented_in(view)?;
                let tab = session.tab(tab_id)?;
                let layout = connection
                    .zoomed_pane(tab)
                    .map(PaneLayout::Pane)
                    .or_else(|| tab.layout().cloned());
                let viewer = match tab.content() {
                    TabContent::Diff(diff) => Some(diff.path().to_relative_path_buf()),
                    TabContent::File(file) => Some(file.path().to_relative_path_buf()),
                    TabContent::Terminals(_) => None,
                };
                Some((tab_id, layout, viewer))
            })
            .collect()
    }

    /// Whether any window presents connection `key`, so its structure changes reach a Dock.
    pub(super) fn presents(&self, key: ConnectionKey) -> bool {
        self.windows
            .iter()
            .any(|view| self.presented_key(view) == key)
    }

    /// Brings `handle` forward among this Client's windows, never Condr in front of another
    /// application: a click on an OS notification has done that already.
    pub(super) fn activate_window(&self, handle: AnyWindowHandle, cx: &mut App) {
        let Some(active) = cx.active_window() else {
            return;
        };
        if active == handle || !self.windows.iter().any(|view| view.handle == active) {
            return;
        }
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, _| window.activate_window());
        });
    }

    /// Rebuilds every window's Dock: `window`'s now, the others' once this update ends.
    pub(super) fn rebuild_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = window.window_handle();
        if self.owns_window(window) {
            self.rebuild_window_dock(window, cx);
        }
        for handle in self.windows.iter().map(|view| view.handle) {
            if handle == current {
                continue;
            }
            let owner = cx.weak_entity();
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, cx| {
                    owner.update(cx, |this, cx| {
                        if this.owns_window(window) {
                            this.rebuild_window_dock(window, cx);
                            this.sync_terminal_focus(window, cx);
                            cx.notify();
                        }
                    })
                });
            });
        }
    }

    /// "Open in New Window": `scope`'s unit moves to a window of its own, cascaded from
    /// `window`, and the place it left keeps it selected and shows the page saying where
    /// it went.
    pub(super) fn open_in_new_window(
        &mut self,
        scope: WindowScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_in_new_window_at(scope, point(px(32.), px(32.)), window, cx);
    }

    /// Opens `scope`'s window with its top-left corner at `position` in `window`: cascaded
    /// from it from the menu, at the cursor when dragged out.
    pub(super) fn open_in_new_window_at(
        &mut self,
        scope: WindowScope,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(handle) = self.window_for(scope) {
            self.activate_window(handle, cx);
            return;
        }
        let from_main = self.is_main(window);
        match scope {
            WindowScope::Main => return,
            WindowScope::Device(key) => {
                if from_main {
                    self.active_connection = key;
                }
            }
            WindowScope::Workspace(key, workspace_id) => {
                self.show_view(key, workspace_id, None);
                if from_main {
                    self.active_connection = key;
                }
            }
            WindowScope::Tab(key, tab_id) => {
                let Some(workspace_id) = self
                    .connection(key)
                    .and_then(ServerConnection::session)
                    .and_then(|session| workspace_of_tab(session, tab_id))
                else {
                    return;
                };
                self.show_view(key, workspace_id, Some(tab_id));
                if from_main {
                    self.active_connection = key;
                }
            }
        }
        let size = Bounds::<Pixels>::from(self.main_view().bounds.rect()).size;
        let bounds = Bounds::new(window.bounds().origin + position, size);
        self.open_window(
            scope,
            WindowBounds::Windowed(bounds),
            window.display(cx).map(|display| display.id()),
            cx,
        );
        self.state_dirty = true;
        self.rebuild_dock(window, cx);
        cx.notify();
    }

    /// Opens a detached window once the current update ends, since its first frame draws
    /// `Condr`.
    pub(super) fn open_window(
        &self,
        scope: WindowScope,
        bounds: WindowBounds,
        display_id: Option<DisplayId>,
        cx: &mut Context<Self>,
    ) {
        let owner = cx.entity();
        let (sidebar_width, sidebar_collapsed, changes_width) = {
            let main = self.main_view();
            (
                main.sidebar_width,
                main.sidebar_collapsed,
                main.changes_width,
            )
        };
        cx.defer(move |cx| {
            let options = WindowOptions {
                window_bounds: Some(bounds),
                display_id,
                ..crate::assets::window_options()
            };
            let opened = cx.open_window(options, |window, cx| {
                owner.update(cx, |this, cx| {
                    let mut view = WindowView::new(window, scope, cx);
                    view.sidebar_width = sidebar_width;
                    view.sidebar_collapsed = sidebar_collapsed;
                    view.changes_width = changes_width;
                    this.windows.push(view);
                    window.set_window_title(&this.window_title(scope));
                    let close_owner = cx.weak_entity();
                    window.on_window_should_close(cx, move |window, cx| {
                        let _ = close_owner.update(cx, |this, cx| this.bring_back(window, cx));
                        true
                    });
                    // Every window, since the one it left drew it until now.
                    let rebuild_owner = cx.weak_entity();
                    window.defer(cx, move |window, cx| {
                        let _ = rebuild_owner.update(cx, |this, cx| {
                            this.rebuild_dock(window, cx);
                            cx.notify();
                        });
                    });
                });
                cx.new(|cx| Root::new(owner.clone(), window, cx))
            });
            if let Err(error) = opened {
                tracing::warn!("failed to open a window: {error:#}");
            }
        });
    }

    /// Closing a detached window, or its Bring Back button, returns its unit to the
    /// nearest window that has its ancestor, where it becomes the shown item.
    pub(super) fn bring_back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let handle = window.window_handle();
        let Some(index) = self
            .windows
            .iter()
            .position(|view| view.handle == handle)
            .filter(|index| *index > 0)
        else {
            return;
        };
        let view = self.windows.remove(index);
        if self.last_active == handle {
            self.last_active = self.main_window();
        }
        match view.scope {
            WindowScope::Main => {}
            WindowScope::Device(key) => {
                if self.home_window(key, None, None) == self.main_window() {
                    self.active_connection = key;
                }
            }
            WindowScope::Workspace(key, workspace_id) => {
                self.show_view(key, workspace_id, None);
            }
            WindowScope::Tab(key, tab_id) => {
                if let Some(workspace_id) = self
                    .connection(key)
                    .and_then(ServerConnection::session)
                    .and_then(|session| workspace_of_tab(session, tab_id))
                {
                    self.show_view(key, workspace_id, Some(tab_id));
                }
            }
        }
        self.state_dirty = true;
        self.rebuild_dock(window, cx);
        cx.notify();
    }

    /// "Bring Back" in a detached window's title bar: the same as closing it.
    pub(super) fn bring_back_and_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.bring_back(window, cx);
        window.remove_window();
    }

    /// A detached window whose unit is gone closes itself: its Tab or Workspace closed,
    /// or a Bootstrap no longer has its id. A Device that merely disconnected keeps its
    /// windows, which show its connection state.
    pub(super) fn close_windows_without_units(&mut self, key: ConnectionKey, cx: &mut App) {
        let Some(session) = self.connection(key).and_then(ServerConnection::session) else {
            return;
        };
        let gone: Vec<_> = self
            .windows
            .iter()
            .filter(|view| match view.scope {
                WindowScope::Workspace(scope_key, workspace_id) => {
                    scope_key == key && session.workspace(workspace_id).is_none()
                }
                WindowScope::Tab(scope_key, tab_id) => {
                    scope_key == key && session.tab(tab_id).is_none()
                }
                WindowScope::Main | WindowScope::Device(_) => false,
            })
            .map(|view| view.handle)
            .collect();
        self.close_windows(&gone, cx);
    }

    /// Closes detached windows without bringing anything back.
    pub(super) fn close_windows(&mut self, handles: &[AnyWindowHandle], cx: &mut App) {
        if handles.is_empty() {
            return;
        }
        self.windows
            .retain(|view| !handles.contains(&view.handle) || view.scope == WindowScope::Main);
        if handles.contains(&self.last_active) {
            self.last_active = self.main_window();
        }
        self.state_dirty = true;
        for handle in handles.iter().copied() {
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            });
        }
    }

    /// `Condr – <name>`, the OS title of a detached window and its title bar's text.
    pub(super) fn window_title(&self, scope: WindowScope) -> String {
        let name = match scope {
            WindowScope::Main => None,
            WindowScope::Device(key) => self
                .connection(key)
                .map(|connection| connection.label.to_string()),
            WindowScope::Workspace(key, workspace_id) => self
                .connection(key)
                .and_then(ServerConnection::session)
                .and_then(|session| session.workspace(workspace_id))
                .map(|workspace| workspace.name().to_owned()),
            WindowScope::Tab(key, tab_id) => self
                .connection(key)
                .and_then(ServerConnection::session)
                .and_then(|session| {
                    let workspace = session.workspace(workspace_of_tab(session, tab_id)?)?;
                    let index = workspace.tabs().iter().position(|tab| tab.id() == tab_id)?;
                    let tab = &workspace.tabs()[index];
                    Some(format!(
                        "{} · {}",
                        workspace.name(),
                        workspace::tab_label(index, tab.name())
                    ))
                }),
        };
        match name {
            Some(name) => format!("{} – {name}", condr_core::APP_NAME),
            None => condr_core::APP_NAME.to_owned(),
        }
    }
}

pub(super) fn workspace_of_tab(session: &Session, tab_id: TabId) -> Option<WorkspaceId> {
    session
        .workspaces()
        .iter()
        .find(|workspace| workspace.tab(tab_id).is_some())
        .map(Workspace::id)
}
