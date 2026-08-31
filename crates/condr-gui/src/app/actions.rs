use super::*;

impl Condr {
    pub(super) fn action_add_server(
        &mut self,
        _: &AddServer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.prompt_add_server(window, cx);
    }

    pub(super) fn action_reconnect(
        &mut self,
        _: &ReconnectServer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.reconnect_active(window, cx);
        cx.notify();
    }

    pub(super) fn action_new_workspace(
        &mut self,
        _: &NewWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.choose_workspace_directory_on(self.active_connection, window, cx);
    }

    pub(super) fn action_new_tab(
        &mut self,
        _: &NewTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.new_tab(window, cx);
    }

    pub(super) fn action_rename_workspace(
        &mut self,
        _: &RenameWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.prompt_rename_workspace(window, cx);
    }

    pub(super) fn action_rename_tab(
        &mut self,
        _: &RenameTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.prompt_rename_tab(window, cx);
    }

    pub(super) fn action_move_workspace_up(
        &mut self,
        _: &MoveWorkspaceUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.move_workspace(-1);
    }

    pub(super) fn action_move_workspace_down(
        &mut self,
        _: &MoveWorkspaceDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.move_workspace(1);
    }

    pub(super) fn action_move_tab_left(
        &mut self,
        _: &MoveTabLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.move_tab(-1);
    }

    pub(super) fn action_move_tab_right(
        &mut self,
        _: &MoveTabRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.move_tab(1);
    }

    pub(super) fn action_close_pane(
        &mut self,
        _: &ClosePane,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.close_pane(window, cx);
    }

    pub(super) fn action_close_tab(
        &mut self,
        _: &CloseTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.close_tab(window, cx);
    }

    pub(super) fn action_close_workspace(
        &mut self,
        _: &CloseWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.close_workspace(window, cx);
    }

    pub(super) fn action_next_tab(
        &mut self,
        _: &NextTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.cycle_tab(1, window, cx);
    }

    pub(super) fn action_previous_tab(
        &mut self,
        _: &PreviousTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.cycle_tab(-1, window, cx);
    }

    pub(super) fn action_split_right(
        &mut self,
        _: &SplitRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.split(SplitDirection::Horizontal);
    }

    pub(super) fn action_split_down(
        &mut self,
        _: &SplitDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.split(SplitDirection::Vertical);
    }

    pub(super) fn action_focus_left(
        &mut self,
        _: &FocusLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.focus_direction(PaneDirection::Left);
    }

    pub(super) fn action_focus_right(
        &mut self,
        _: &FocusRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.focus_direction(PaneDirection::Right);
    }

    pub(super) fn action_focus_up(
        &mut self,
        _: &FocusUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.focus_direction(PaneDirection::Up);
    }

    pub(super) fn action_focus_down(
        &mut self,
        _: &FocusDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.focus_direction(PaneDirection::Down);
    }

    pub(super) fn action_resize_left(
        &mut self,
        _: &ResizeLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.resize_direction(PaneDirection::Left);
    }

    pub(super) fn action_resize_right(
        &mut self,
        _: &ResizeRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.resize_direction(PaneDirection::Right);
    }

    pub(super) fn action_resize_up(
        &mut self,
        _: &ResizeUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.resize_direction(PaneDirection::Up);
    }

    pub(super) fn action_resize_down(
        &mut self,
        _: &ResizeDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.resize_direction(PaneDirection::Down);
    }

    pub(super) fn action_swap_left(
        &mut self,
        _: &SwapLeft,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.swap_direction(PaneDirection::Left);
    }

    pub(super) fn action_swap_right(
        &mut self,
        _: &SwapRight,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.swap_direction(PaneDirection::Right);
    }

    pub(super) fn action_swap_up(
        &mut self,
        _: &SwapUp,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.swap_direction(PaneDirection::Up);
    }

    pub(super) fn action_swap_down(
        &mut self,
        _: &SwapDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.swap_direction(PaneDirection::Down);
    }

    pub(super) fn action_toggle_zoom(
        &mut self,
        _: &ToggleZoom,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_dialog(window, cx);
        self.toggle_zoom();
    }
}
