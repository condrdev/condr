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
    ) {
        self.terminal_geometry
            .insert((key, pane_id), TerminalGeometry { bounds, cell_size });
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
        {
            selection.dragging = false;
        }
    }

    pub(super) fn selection_for(
        &self,
        key: ConnectionKey,
        pane_id: PaneId,
    ) -> Option<TerminalSelection> {
        self.terminal_selection
            .filter(|selection| selection.connection_key == key && selection.pane_id == pane_id)
            .map(|selection| selection.range)
    }

    pub(super) fn copy_terminal_selection(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(selection) = self.selection_for(key, pane_id) else {
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

        if self.terminal_command(key, pane_id, TerminalCommand::Copy { selection }) {
            self.clear_selection(cx);
            true
        } else {
            false
        }
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

    pub(crate) fn scroll_terminal(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        lines: i32,
        cx: &mut Context<Self>,
    ) {
        if lines != 0 {
            self.clear_selection(cx);
            self.terminal_command(
                key,
                pane_id,
                TerminalCommand::Scroll(TerminalScroll::Lines(lines)),
            );
        }
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
        if let Some(action) = fixed_shortcut(stroke) {
            window.dispatch_action(action, cx);
            cx.stop_propagation();
            return;
        }
        let terminal_focused = self
            .panels
            .get(&(key, pane_id))
            .map(|panel| panel.read(cx).focus_handle.clone())
            .is_some_and(|focus| focus.is_focused(window));
        if !terminal_focused {
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
        let scroll = match stroke.key.as_str() {
            "pageup" if modifiers.shift => Some(TerminalScroll::PageUp),
            "pagedown" if modifiers.shift => Some(TerminalScroll::PageDown),
            _ => None,
        };
        if let Some(scroll) = scroll {
            self.clear_selection(cx);
            self.terminal_command(key, pane_id, TerminalCommand::Scroll(scroll));
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
            self.clear_selection(cx);
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
        "c" if command_only || control_shift || (control_only && has_selection) => {
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
