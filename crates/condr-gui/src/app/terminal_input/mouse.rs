use super::*;

impl Condr {
    pub(crate) fn start_terminal_mouse_capture(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        button: TerminalMouseButton,
        event: TerminalMouseEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.terminal_command(key, pane_id, TerminalCommand::Mouse(event)) {
            return false;
        }
        self.clear_selection(cx);
        self.last_terminal_mouse_motion = None;
        self.terminal_mouse_capture = Some(ReportedTerminalMouse {
            connection_key: key,
            pane_id,
            button,
        });
        true
    }

    pub(crate) fn terminal_mouse_capture(
        &self,
        key: ConnectionKey,
        pane_id: PaneId,
    ) -> Option<TerminalMouseButton> {
        self.terminal_mouse_capture
            .filter(|capture| capture.connection_key == key && capture.pane_id == pane_id)
            .map(|capture| capture.button)
    }

    pub(crate) fn terminal_mouse_gesture_owner(&self) -> Option<(ConnectionKey, PaneId)> {
        self.terminal_selection
            .filter(|selection| selection.dragging)
            .map(|selection| (selection.connection_key, selection.pane_id))
            .or_else(|| {
                self.terminal_mouse_capture
                    .map(|capture| (capture.connection_key, capture.pane_id))
            })
    }

    pub(crate) fn report_captured_terminal_mouse(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        event: TerminalMouseEvent,
    ) -> bool {
        if self.terminal_mouse_capture(key, pane_id).is_none() {
            return false;
        }
        self.report_terminal_motion(key, pane_id, event)
    }

    pub(crate) fn finish_terminal_mouse_capture(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        button: TerminalMouseButton,
        event: TerminalMouseEvent,
    ) -> bool {
        if self.terminal_mouse_capture(key, pane_id) != Some(button) {
            return false;
        }
        self.terminal_mouse_capture = None;
        self.last_terminal_mouse_motion = None;
        self.terminal_command(key, pane_id, TerminalCommand::Mouse(event));
        true
    }

    pub(crate) fn report_terminal_motion(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        event: TerminalMouseEvent,
    ) -> bool {
        let Some(mouse_tracking) = self
            .terminal(key, pane_id)
            .map(|terminal| terminal.view.mouse_tracking)
        else {
            return false;
        };
        let motion = ReportedTerminalMouseMotion {
            connection_key: key,
            pane_id,
            mouse_tracking,
            event,
        };
        if self.last_terminal_mouse_motion == Some(motion) {
            return true;
        }
        if self.terminal_command(key, pane_id, TerminalCommand::Mouse(event)) {
            self.last_terminal_mouse_motion = Some(motion);
            true
        } else {
            false
        }
    }

    pub(crate) fn report_terminal_mouse(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        event: TerminalMouseEvent,
        clear_selection: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        self.last_terminal_mouse_motion = None;
        let sent = self.terminal_command(key, pane_id, TerminalCommand::Mouse(event));
        if sent && clear_selection {
            self.clear_selection(cx);
        }
        sent
    }

    pub(crate) fn set_hovered_link(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        link: Option<HoveredTerminalLink>,
        cx: &mut Context<Self>,
    ) {
        if let Some(next) = next_hovered_link(self.hovered_link.as_ref(), key, pane_id, link) {
            self.hovered_link = next;
            cx.notify();
        }
    }

    pub(crate) fn hovered_link_for(
        &self,
        key: ConnectionKey,
        pane_id: PaneId,
    ) -> Option<HoveredTerminalLink> {
        self.hovered_link
            .as_ref()
            .filter(|(link_key, link_pane, _)| *link_key == key && *link_pane == pane_id)
            .map(|(_, _, link)| link.clone())
    }

    pub(crate) fn set_pressed_terminal_link(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        link: Option<HoveredTerminalLink>,
    ) {
        self.pressed_terminal_link = link.map(|link| (key, pane_id, link));
    }

    pub(crate) fn take_pressed_terminal_link(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
    ) -> Option<HoveredTerminalLink> {
        if self
            .pressed_terminal_link
            .as_ref()
            .is_some_and(|(link_key, link_pane, _)| *link_key == key && *link_pane == pane_id)
        {
            self.pressed_terminal_link.take().map(|(_, _, link)| link)
        } else {
            None
        }
    }
}

/// The hover state after a pane reports `link`, or `None` when nothing changes. Every pane's
/// mouse handler reports, so a `None` from a pane that does not own the current hover must
/// not clear the pane that does.
pub(in crate::app) fn next_hovered_link(
    current: Option<&(ConnectionKey, PaneId, HoveredTerminalLink)>,
    key: ConnectionKey,
    pane_id: PaneId,
    link: Option<HoveredTerminalLink>,
) -> Option<Option<(ConnectionKey, PaneId, HoveredTerminalLink)>> {
    match link {
        Some(link) => {
            let next = (key, pane_id, link);
            (current != Some(&next)).then_some(Some(next))
        }
        None => current
            .is_some_and(|(owner_key, owner_pane, _)| *owner_key == key && *owner_pane == pane_id)
            .then_some(None),
    }
}

#[derive(Clone, Copy)]
pub(in crate::app) struct ReportedTerminalMouse {
    pub(in crate::app) connection_key: ConnectionKey,
    pub(in crate::app) pane_id: PaneId,
    pub(in crate::app) button: TerminalMouseButton,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(in crate::app) struct ReportedTerminalMouseMotion {
    pub(in crate::app) connection_key: ConnectionKey,
    pub(in crate::app) pane_id: PaneId,
    pub(in crate::app) mouse_tracking: TerminalMouseTracking,
    pub(in crate::app) event: TerminalMouseEvent,
}
