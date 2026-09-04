use super::*;

impl Condr {
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
        if self.terminal_command(key, pane_id, TerminalCommand::Resize(size)) {
            self.pending_sizes.insert(pending_key, size);
            cx.notify();
        }
    }

    pub(crate) fn update_terminal_geometry(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        bounds: Bounds<Pixels>,
        cell_size: Size<Pixels>,
        window: &Window,
    ) {
        let geometry = TerminalGeometry { bounds, cell_size };
        let changed = self.terminal_geometry.get(&(key, pane_id)) != Some(&geometry);
        self.terminal_geometry.insert((key, pane_id), geometry);
        if changed
            && self
                .terminal_composition
                .as_ref()
                .is_some_and(|composition| {
                    composition.belongs_to_target(self.target_pane)
                        && composition.belongs_to(key, pane_id)
                })
        {
            window.invalidate_character_coordinates();
        }
    }

    pub(crate) fn terminal_layout_is_current(
        &self,
        key: ConnectionKey,
        pane_id: PaneId,
        bounds: Bounds<Pixels>,
        cell_size: Size<Pixels>,
        terminal_size: TerminalSize,
    ) -> bool {
        self.terminal_geometry.get(&(key, pane_id)) == Some(&TerminalGeometry { bounds, cell_size })
            && (self.pending_sizes.get(&(key, pane_id)) == Some(&terminal_size)
                || self
                    .terminal(key, pane_id)
                    .is_some_and(|terminal| terminal.view.size == terminal_size))
    }

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

    pub(crate) fn sync_terminal_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !window.is_window_active() {
            window.release_pointer();
            self.terminal_mouse_capture = None;
            if let Some(selection) = &mut self.terminal_selection {
                selection.dragging = false;
            }
        }
        let focused = window
            .is_window_active()
            .then(|| {
                self.panels.iter().find_map(|(&(key, pane_id), panel)| {
                    panel
                        .read(cx)
                        .focus_handle
                        .is_focused(window)
                        .then_some((key, pane_id))
                })
            })
            .flatten();
        if focused != self.focused_terminal {
            self.focused_terminal = focused;
            if let Some((key, pane_id)) = focused
                && self.clear_pane_attention(key, pane_id)
            {
                cx.notify();
            }
        }
        let reportable_focus = focused.filter(|(key, _)| {
            self.connection(*key)
                .is_some_and(|connection| connection.controlling)
        });
        if reportable_focus == self.reported_terminal_focus {
            return;
        }
        self.last_terminal_mouse_motion = None;

        if let Some((key, pane_id)) = self.reported_terminal_focus.take() {
            self.terminal_command(key, pane_id, TerminalCommand::Focus(false));
        }
        if let Some((key, pane_id)) = reportable_focus
            && self.terminal_command(key, pane_id, TerminalCommand::Focus(true))
        {
            self.reported_terminal_focus = Some((key, pane_id));
        }
    }

    pub(crate) fn begin_selection(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        position: TerminalPosition,
        click_count: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(terminal) = self.terminal(key, pane_id) else {
            return;
        };
        let display_offset = terminal.view.display_offset;
        let multi_click_range = match click_count {
            2 => terminal
                .view
                .word_selection_at(position.row, position.column),
            3.. => terminal.view.line_selection_at(position.row),
            _ => None,
        };
        self.terminal_selection = Some(LocalTerminalSelection {
            connection_key: key,
            pane_id,
            range: multi_click_range.unwrap_or(TerminalSelection {
                start: position,
                end: position,
                display_offset,
            }),
            dragging: click_count == 1,
        });
        // A single click clears the Server-tracked selection; a word or line becomes it.
        self.terminal_command(key, pane_id, TerminalCommand::Select(multi_click_range));
        cx.notify();
    }

    pub(crate) fn update_selection(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        position: TerminalPosition,
        cx: &mut Context<Self>,
    ) {
        if let Some(selection) = &mut self.terminal_selection
            && selection.dragging
            && selection.connection_key == key
            && selection.pane_id == pane_id
            && selection.range.end != position
        {
            selection.range.end = position;
            cx.notify();
        }
    }

    pub(crate) fn is_selecting(&self, key: ConnectionKey, pane_id: PaneId) -> bool {
        self.terminal_selection.is_some_and(|selection| {
            selection.dragging && selection.connection_key == key && selection.pane_id == pane_id
        })
    }

    pub(crate) fn end_selection(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        position: TerminalPosition,
        cx: &mut Context<Self>,
    ) {
        self.update_selection(key, pane_id, position, cx);
        if let Some(selection) = &mut self.terminal_selection
            && selection.connection_key == key
            && selection.pane_id == pane_id
            && selection.dragging
        {
            selection.dragging = false;
            let range = selection.range;
            let columns = self
                .terminal(key, pane_id)
                .map_or(0, |terminal| terminal.view.size.columns);
            // The Server keeps the finished drag attached to its text; the local copy
            // stays as the fallback for a Client that may not mutate.
            self.terminal_command(
                key,
                pane_id,
                TerminalCommand::Select(range.selected_cell_range(columns).map(|_| range)),
            );
        }
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

    /// The selection to draw and copy: the local one while dragging, otherwise the
    /// Server-tracked one, which follows the text as output scrolls.
    pub(super) fn selection_for(
        &self,
        key: ConnectionKey,
        pane_id: PaneId,
    ) -> Option<TerminalSelection> {
        self.effective_selection(key, pane_id)
            .map(|(selection, _)| selection)
    }

    /// The selection and whether the Server tracks it.
    fn effective_selection(
        &self,
        key: ConnectionKey,
        pane_id: PaneId,
    ) -> Option<(TerminalSelection, bool)> {
        let local = self
            .terminal_selection
            .filter(|selection| selection.connection_key == key && selection.pane_id == pane_id);
        if let Some(local) = local.filter(|selection| selection.dragging) {
            return Some((local.range, false));
        }
        self.terminal(key, pane_id)
            .and_then(|terminal| terminal.view.selection)
            .map(|selection| (selection, true))
            .or(local.map(|local| (local.range, false)))
    }

    pub(super) fn copy_terminal_selection(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        _cx: &mut Context<Self>,
    ) -> bool {
        let Some((selection, server_tracked)) = self.effective_selection(key, pane_id) else {
            return false;
        };
        let Some(columns) = self
            .terminal(key, pane_id)
            .map(|terminal| terminal.view.size.columns)
        else {
            return false;
        };
        if selection.selected_cell_range(columns).is_none() {
            return false;
        }

        // The Server's own selection also covers rows scrolled out of the viewport.
        let selection = (!server_tracked).then_some(selection);
        self.terminal_command(key, pane_id, TerminalCommand::Copy { selection })
    }

    pub(super) fn paste_into_terminal(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        self.clear_selection(cx);
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.terminal_command(key, pane_id, TerminalCommand::Paste(text));
        }
    }

    pub(super) fn clear_selection(&mut self, cx: &mut Context<Self>) {
        if self.terminal_selection.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn action_terminal_tab(
        &mut self,
        _: &TerminalTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_terminal_action_key(TerminalKey::Tab, TerminalModifiers::default(), window, cx);
    }

    pub(super) fn action_terminal_back_tab(
        &mut self,
        _: &TerminalBackTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_terminal_action_key(
            TerminalKey::BackTab,
            TerminalModifiers {
                shift: true,
                ..TerminalModifiers::default()
            },
            window,
            cx,
        );
    }

    fn send_terminal_action_key(
        &mut self,
        terminal_key: TerminalKey,
        modifiers: TerminalModifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((key, pane_id)) = self.target_pane else {
            cx.propagate();
            return;
        };
        if !self.terminal_is_focused(key, pane_id, window, cx) {
            cx.propagate();
            return;
        }

        self.clear_selection(cx);
        self.terminal_command(
            key,
            pane_id,
            TerminalCommand::Key {
                key: terminal_key,
                modifiers,
            },
        );
    }

    fn terminal_is_focused(
        &self,
        key: ConnectionKey,
        pane_id: PaneId,
        window: &Window,
        cx: &App,
    ) -> bool {
        self.panels
            .get(&(key, pane_id))
            .map(|panel| panel.read(cx).focus_handle.clone())
            .is_some_and(|focus| focus.is_focused(window))
    }

    pub(super) fn key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((key, pane_id)) = self.target_pane else {
            return;
        };
        let stroke = &event.keystroke;
        if should_defer_to_character_input(event) {
            return;
        }
        if let Some(action) = fixed_shortcut(stroke) {
            window.dispatch_action(action, cx);
            cx.stop_propagation();
            return;
        }
        if !self.terminal_is_focused(key, pane_id, window, cx) {
            return;
        }
        let modifiers = stroke.modifiers;
        let has_selection = self
            .selection_for(key, pane_id)
            .and_then(|selection| {
                let columns = self.terminal(key, pane_id)?.view.size.columns;
                selection.selected_cell_range(columns)
            })
            .is_some();
        if let Some(shortcut) = terminal_clipboard_shortcut(stroke, has_selection) {
            match shortcut {
                TerminalClipboardShortcut::Copy => {
                    self.copy_terminal_selection(key, pane_id, cx);
                }
                TerminalClipboardShortcut::Paste => {
                    self.paste_into_terminal(key, pane_id, cx);
                }
            }
            cx.stop_propagation();
            return;
        }
        if modifiers.platform && !modifiers.control && !modifiers.alt {
            // Cmd/Win chords without a binding are never terminal input; let them propagate.
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
                .filter(|number| (1..=20).contains(number))
                .map(TerminalKey::Function),
            _ if modifiers.control || modifiers.alt => Some(TerminalKey::Character(
                stroke
                    .key_char
                    .clone()
                    .unwrap_or_else(|| stroke.key.clone()),
            )),
            _ => None,
        };
        if let Some(key_code) = key_code {
            self.clear_selection(cx);
            self.restart_cursor_blink(key, pane_id, cx);
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
}

pub(super) fn should_defer_to_character_input(event: &KeyDownEvent) -> bool {
    event.prefer_character_input
        && event
            .keystroke
            .key_char
            .as_deref()
            .is_some_and(|text| !text.is_empty() && text.chars().all(|ch| !ch.is_control()))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TerminalClipboardShortcut {
    Copy,
    Paste,
}

pub(super) fn terminal_clipboard_shortcut(
    stroke: &Keystroke,
    has_selection: bool,
) -> Option<TerminalClipboardShortcut> {
    let modifiers = stroke.modifiers;
    if modifiers.alt || modifiers.function {
        return None;
    }

    let command_only =
        cfg!(target_os = "macos") && modifiers.platform && !modifiers.control && !modifiers.shift;
    let control_only = modifiers.control && !modifiers.platform && !modifiers.shift;
    let control_shift = modifiers.control && !modifiers.platform && modifiers.shift;
    let shift_only = modifiers.shift && !modifiers.control && !modifiers.platform;

    match stroke.key.as_str() {
        "c" if command_only
            || control_shift
            || (cfg!(windows) && control_only && has_selection) =>
        {
            Some(TerminalClipboardShortcut::Copy)
        }
        "v" if command_only || control_shift || (cfg!(windows) && control_only) => {
            Some(TerminalClipboardShortcut::Paste)
        }
        "insert" if control_only => Some(TerminalClipboardShortcut::Copy),
        "insert" if shift_only => Some(TerminalClipboardShortcut::Paste),
        _ => None,
    }
}

/// The hover state after a pane reports `link`, or `None` when nothing changes. Every pane's
/// mouse handler reports, so a `None` from a pane that does not own the current hover must
/// not clear the pane that does.
pub(super) fn next_hovered_link(
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
