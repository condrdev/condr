use super::*;

/// What one of this Client's windows keeps for itself (ADR 0043); the connections, the
/// Panes, the Dock surfaces and every Workspace's view are the application's, so they
/// follow whatever moves between windows. `Condr` is the root view of each window and
/// finds its own by the window it draws in.
pub(super) struct WindowView {
    pub(super) handle: AnyWindowHandle,
    pub(super) focus_handle: FocusHandle,
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
}

impl WindowView {
    pub(super) fn new(
        window: &Window,
        restored: &gui_state::GuiState,
        cx: &mut Context<Condr>,
    ) -> Self {
        Self {
            handle: window.window_handle(),
            focus_handle: cx.focus_handle(),
            workspace_size: size(
                (window.viewport_size().width - INITIAL_SIDEBAR_WIDTH).max(px(0.)),
                (window.viewport_size().height - WORKSPACE_TITLE_BAR_HEIGHT).max(px(0.)),
            ),
            sidebar_width: restored
                .sidebar_width
                .map_or(INITIAL_SIDEBAR_WIDTH, |width| {
                    px(width).max(MIN_SIDEBAR_WIDTH).min(MAX_SIDEBAR_WIDTH)
                }),
            sidebar_collapsed: restored.sidebar_collapsed,
            changes_width: restored
                .changes_width
                .map_or(INITIAL_CHANGES_WIDTH, |width| {
                    px(width).max(MIN_CHANGES_WIDTH).min(MAX_CHANGES_WIDTH)
                }),
            tab_strip_scroll: ScrollHandle::new(),
            last_scrolled_tab: Cell::new(None),
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
    pub(super) fn window_view(&self, window: &Window) -> &WindowView {
        let id = window.window_handle().window_id();
        self.windows
            .iter()
            .find(|view| view.handle.window_id() == id)
            .expect("Condr draws only in its own windows")
    }

    pub(super) fn window_view_mut(&mut self, window: &Window) -> &mut WindowView {
        let id = window.window_handle().window_id();
        self.windows
            .iter_mut()
            .find(|view| view.handle.window_id() == id)
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
}
