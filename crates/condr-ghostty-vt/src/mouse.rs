//! Mouse reports: libghostty-vt encodes a mouse event for the tracking mode (X10, normal,
//! button or any motion) and format (X10, UTF-8, SGR, URXVT or SGR pixels) the program
//! turned on.

use super::{Error, Mods, Terminal, check, encoded, sys};
use std::mem::size_of;
use std::ptr;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
    WheelLeft,
    WheelRight,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseAction {
    Press,
    Release,
    Motion,
}

/// One mouse event, at a cell of the viewport.
#[derive(Clone, Copy, Debug)]
pub struct MouseEvent {
    pub action: MouseAction,
    /// The button pressed or released, or held during a motion.
    pub button: Option<MouseButton>,
    pub mods: Mods,
    pub column: u16,
    pub row: u16,
}

pub struct MouseEncoder {
    raw: sys::GhosttyMouseEncoder,
    event: sys::GhosttyMouseEvent,
}

// SAFETY: as for `Terminal`: used only through `&mut self`, beside its terminal under one
// mutex.
unsafe impl Send for MouseEncoder {}

impl MouseEncoder {
    pub fn new() -> Result<Self, Error> {
        let mut encoder = Self {
            raw: ptr::null_mut(),
            event: ptr::null_mut(),
        };
        // SAFETY: default allocator; a handle is set only by a call that succeeded, and
        // `Drop` frees whichever were created.
        unsafe {
            check(sys::ghostty_mouse_encoder_new(
                ptr::null(),
                &mut encoder.raw,
            ))?;
            check(sys::ghostty_mouse_event_new(
                ptr::null(),
                &mut encoder.event,
            ))?;
        }
        Ok(encoder)
    }

    /// What `event` reports to the program for `terminal`'s current modes; empty when the
    /// program does not track it. Condr addresses the mouse by cell, so an event sits in
    /// the middle of its cell and an SGR-pixel report is exact to the cell. Every motion
    /// is reported: the Client sends one only when the cell changes.
    pub fn encode(&mut self, terminal: &Terminal, event: &MouseEvent) -> Vec<u8> {
        use sys::GhosttyMouseEncoderOption as Opt;
        let cells = terminal.callbacks().size;
        let (cell_width, cell_height) = (cells.cell_width.max(1), cells.cell_height.max(1));
        let size = sys::GhosttyMouseEncoderSize {
            size: size_of::<sys::GhosttyMouseEncoderSize>(),
            screen_width: u32::from(cells.columns) * cell_width,
            screen_height: u32::from(cells.rows) * cell_height,
            cell_width,
            cell_height,
            padding_top: 0,
            padding_bottom: 0,
            padding_right: 0,
            padding_left: 0,
        };
        let pressed = event.button.is_some() && event.action != MouseAction::Release;
        let position = sys::GhosttyMousePosition {
            x: (f32::from(event.column) + 0.5) * cell_width as f32,
            y: (f32::from(event.row) + 0.5) * cell_height as f32,
        };
        let action = match event.action {
            MouseAction::Press => sys::GhosttyMouseAction::GHOSTTY_MOUSE_ACTION_PRESS,
            MouseAction::Release => sys::GhosttyMouseAction::GHOSTTY_MOUSE_ACTION_RELEASE,
            MouseAction::Motion => sys::GhosttyMouseAction::GHOSTTY_MOUSE_ACTION_MOTION,
        };
        // SAFETY: live handles and values of the types each option names.
        unsafe {
            sys::ghostty_mouse_encoder_setopt_from_terminal(self.raw, terminal.raw);
            sys::ghostty_mouse_encoder_setopt(
                self.raw,
                Opt::GHOSTTY_MOUSE_ENCODER_OPT_SIZE,
                (&raw const size).cast(),
            );
            sys::ghostty_mouse_encoder_setopt(
                self.raw,
                Opt::GHOSTTY_MOUSE_ENCODER_OPT_ANY_BUTTON_PRESSED,
                (&raw const pressed).cast(),
            );
            sys::ghostty_mouse_event_set_action(self.event, action);
            match event.button {
                Some(button) => sys::ghostty_mouse_event_set_button(self.event, button.to_sys()),
                None => sys::ghostty_mouse_event_clear_button(self.event),
            }
            sys::ghostty_mouse_event_set_mods(self.event, event.mods.bits());
            sys::ghostty_mouse_event_set_position(self.event, position);
        }
        // SAFETY: live handles; the buffer is as long as said.
        encoded(|buf, len, written| unsafe {
            sys::ghostty_mouse_encoder_encode(self.raw, self.event, buf, len, written)
        })
    }
}

impl MouseButton {
    fn to_sys(self) -> sys::GhosttyMouseButton {
        use sys::GhosttyMouseButton as Button;
        match self {
            Self::Left => Button::GHOSTTY_MOUSE_BUTTON_LEFT,
            Self::Middle => Button::GHOSTTY_MOUSE_BUTTON_MIDDLE,
            Self::Right => Button::GHOSTTY_MOUSE_BUTTON_RIGHT,
            Self::WheelUp => Button::GHOSTTY_MOUSE_BUTTON_FOUR,
            Self::WheelDown => Button::GHOSTTY_MOUSE_BUTTON_FIVE,
            Self::WheelLeft => Button::GHOSTTY_MOUSE_BUTTON_SIX,
            Self::WheelRight => Button::GHOSTTY_MOUSE_BUTTON_SEVEN,
        }
    }
}

impl Drop for MouseEncoder {
    fn drop(&mut self) {
        // SAFETY: created in `new` (a null handle is skipped) and freed only here.
        unsafe {
            if !self.event.is_null() {
                sys::ghostty_mouse_event_free(self.event);
            }
            if !self.raw.is_null() {
                sys::ghostty_mouse_encoder_free(self.raw);
            }
        }
    }
}
