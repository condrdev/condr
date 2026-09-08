mod clipboard;
mod mouse;
mod selection;

use super::*;

#[cfg(test)]
pub(super) use clipboard::clipboard_image_format;
pub(super) use clipboard::{is_image_paste_gesture, prepare_clipboard_image};
#[cfg(test)]
pub(super) use mouse::next_hovered_link;
pub(super) use mouse::{ReportedTerminalMouse, ReportedTerminalMouseMotion};
pub(super) use selection::LocalTerminalSelection;

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
            if let Some((key, pane_id)) = focused {
                // Returning to the window shows this Pane: its completion has been seen.
                self.mark_pane_seen(key, pane_id);
                self.clear_pane_attention(key, pane_id);
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
        if should_defer_to_character_input(event) || window.has_active_dialog(cx) {
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
        if is_image_paste_gesture(stroke)
            && self
                .connection(key)
                .is_some_and(|connection| connection.endpoint.as_local_path().is_none())
            && self.paste_clipboard_image(key, pane_id, cx)
        {
            cx.stop_propagation();
            return;
        }
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
        if modifiers.platform {
            // Cmd/Win chords without a binding are never terminal input (legacy encoding
            // has no way to carry the modifier, so Cmd+Ctrl+C must not become ETX).
            return;
        }
        let key_code = terminal_key_for(stroke);
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

/// The terminal key a GPUI keystroke stands for, or `None` when it is plain text (which
/// arrives through the input handler) or not terminal input at all.
///
/// Platform notes: GPUI's `key` is the unshifted key name on every platform, `key_char` is
/// what typing would produce. macOS fills `key_char` for Option chords ("ß" for option-s),
/// so Option types characters like Terminal.app, iTerm2 and Zed do by default; Windows
/// leaves `key_char` empty while Alt is held, so a shifted letter is uppercased here.
pub(super) fn terminal_key_for(stroke: &Keystroke) -> Option<TerminalKey> {
    let modifiers = stroke.modifiers;
    let named = match stroke.key.as_str() {
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
        _ => None,
    };
    if named.is_some() || !(modifiers.control || modifiers.alt) {
        return named;
    }
    let printable = stroke
        .key_char
        .as_deref()
        .filter(|text| !text.is_empty() && text.chars().all(|ch| !ch.is_control()));
    if cfg!(target_os = "macos") && !modifiers.control && printable.is_some() {
        // Option produced text (ß, €, [ on non-US layouts, or a composed sequence): let
        // it be typed.
        return None;
    }
    let key_char = printable.filter(|text| text.chars().count() == 1);
    let text = match key_char {
        Some(text) => text.to_owned(),
        None => match stroke.key.as_str() {
            "space" => " ".to_owned(),
            key if key.chars().count() == 1 => {
                if modifiers.shift {
                    key.to_ascii_uppercase()
                } else {
                    key.to_owned()
                }
            }
            // capslock, pause, printscreen…: nothing a terminal can encode.
            _ => return None,
        },
    };
    Some(TerminalKey::Character(text))
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
    // Ctrl+Shift+C/V is the Linux/Windows terminal convention; macOS has Cmd and keeps
    // every Ctrl chord for the PTY, as Zed and herdr do.
    let control_shift =
        !cfg!(target_os = "macos") && modifiers.control && !modifiers.platform && modifiers.shift;
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

#[derive(Clone, Copy, PartialEq)]
pub(super) struct TerminalGeometry {
    pub(super) bounds: Bounds<Pixels>,
    pub(super) cell_size: Size<Pixels>,
}
