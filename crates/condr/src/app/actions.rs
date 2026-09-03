use super::*;

macro_rules! action_handlers {
    ($($name:ident($action:ty) |$this:ident, $window:ident, $cx:ident| $body:block)*) => {
        impl Condr {
            $(
                pub(super) fn $name(
                    &mut self,
                    _: &$action,
                    window: &mut Window,
                    cx: &mut Context<Self>,
                ) {
                    self.dismiss_dialog(window, cx);
                    #[allow(unused_variables)]
                    let ($this, $window, $cx) = (self, window, cx);
                    $body
                }
            )*
        }
    };
}

action_handlers! {
    action_add_server(AddServer) |this, window, cx| { this.prompt_add_server(window, cx); }
    action_reconnect(ReconnectServer) |this, window, cx| {
        this.reconnect_active(window, cx);
        cx.notify();
    }
    action_new_workspace(NewWorkspace) |this, window, cx| {
        this.choose_workspace_directory_on(this.active_connection, window, cx);
    }
    action_new_tab(NewTab) |this, window, cx| { this.new_tab(window, cx); }
    action_open_settings(OpenSettings) |this, window, cx| { this.open_settings(window, cx); }
    action_rename_workspace(RenameWorkspace) |this, window, cx| {
        this.prompt_rename_workspace(window, cx);
    }
    action_rename_tab(RenameTab) |this, window, cx| { this.prompt_rename_tab(window, cx); }
    action_close_pane(ClosePane) |this, window, cx| { this.close_pane(window, cx); }
    action_close_tab(CloseTab) |this, window, cx| { this.close_tab(window, cx); }
    action_close_workspace(CloseWorkspace) |this, window, cx| {
        this.close_workspace(window, cx);
    }
    action_next_tab(NextTab) |this, window, cx| { this.cycle_tab(1, window, cx); }
    action_previous_tab(PreviousTab) |this, window, cx| { this.cycle_tab(-1, window, cx); }
    action_split_right(SplitRight) |this, window, cx| {
        this.split(SplitDirection::Horizontal);
    }
    action_split_down(SplitDown) |this, window, cx| { this.split(SplitDirection::Vertical); }
    action_focus_left(FocusLeft) |this, window, cx| { this.focus_direction(PaneDirection::Left); }
    action_focus_right(FocusRight) |this, window, cx| {
        this.focus_direction(PaneDirection::Right);
    }
    action_focus_up(FocusUp) |this, window, cx| { this.focus_direction(PaneDirection::Up); }
    action_focus_down(FocusDown) |this, window, cx| { this.focus_direction(PaneDirection::Down); }
    action_resize_left(ResizeLeft) |this, window, cx| {
        this.resize_direction(PaneDirection::Left);
    }
    action_resize_right(ResizeRight) |this, window, cx| {
        this.resize_direction(PaneDirection::Right);
    }
    action_resize_up(ResizeUp) |this, window, cx| { this.resize_direction(PaneDirection::Up); }
    action_resize_down(ResizeDown) |this, window, cx| {
        this.resize_direction(PaneDirection::Down);
    }
    action_swap_left(SwapLeft) |this, window, cx| { this.swap_direction(PaneDirection::Left); }
    action_swap_right(SwapRight) |this, window, cx| { this.swap_direction(PaneDirection::Right); }
    action_swap_up(SwapUp) |this, window, cx| { this.swap_direction(PaneDirection::Up); }
    action_swap_down(SwapDown) |this, window, cx| { this.swap_direction(PaneDirection::Down); }
    action_toggle_zoom(ToggleZoom) |this, window, cx| { this.toggle_zoom(); }
}
