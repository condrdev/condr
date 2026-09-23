//! Terminal frames and input. The view and delta columns live beside the hyperlink
//! interning in `terminal/view/wire.rs`.

use super::*;
use crate::terminal::{
    decode_position, decode_selection, decode_size, encode_position, encode_selection,
};
use crate::{
    TerminalCommand, TerminalKey, TerminalModifiers, TerminalMouseButton, TerminalMouseEvent,
    TerminalMousePosition, TerminalMouseWheel, TerminalScroll, TerminalSelectionUnit, TerminalView,
    TerminalViewDelta, TerminalViewFrame,
};

impl From<&TerminalViewFrame> for super::TerminalViewFrame {
    fn from(frame: &TerminalViewFrame) -> Self {
        use terminal_view_frame::Frame;
        Self {
            frame: Some(match frame {
                TerminalViewFrame::Full(view) => Frame::Full(view.into()),
                TerminalViewFrame::Delta(delta) => Frame::Delta(delta.into()),
            }),
        }
    }
}

impl TryFrom<super::TerminalViewFrame> for TerminalViewFrame {
    type Error = WireError;

    fn try_from(frame: super::TerminalViewFrame) -> WireResult<Self> {
        use terminal_view_frame::Frame;
        Ok(match member(frame.frame)? {
            Frame::Full(view) => Self::Full(TerminalView::try_from(view)?),
            Frame::Delta(delta) => Self::Delta(TerminalViewDelta::try_from(delta)?),
        })
    }
}

fn encode_modifiers(modifiers: TerminalModifiers) -> super::TerminalModifiers {
    super::TerminalModifiers {
        shift: modifiers.shift,
        alt: modifiers.alt,
        control: modifiers.control,
        platform: modifiers.platform,
    }
}

fn decode_modifiers(modifiers: Option<super::TerminalModifiers>) -> TerminalModifiers {
    let modifiers = modifiers.unwrap_or_default();
    TerminalModifiers {
        shift: modifiers.shift,
        alt: modifiers.alt,
        control: modifiers.control,
        platform: modifiers.platform,
    }
}

fn encode_mouse_position(position: TerminalMousePosition) -> super::TerminalMousePosition {
    super::TerminalMousePosition {
        row: position.row.into(),
        column: position.column.into(),
    }
}

fn decode_mouse_position(
    position: Option<super::TerminalMousePosition>,
) -> WireResult<TerminalMousePosition> {
    let position = required(position, "mouse position")?;
    Ok(TerminalMousePosition {
        row: narrow(position.row, "mouse row")?,
        column: narrow(position.column, "mouse column")?,
    })
}

fn encode_mouse_button(button: TerminalMouseButton) -> i32 {
    (match button {
        TerminalMouseButton::Left => super::TerminalMouseButton::Left,
        TerminalMouseButton::Middle => super::TerminalMouseButton::Middle,
        TerminalMouseButton::Right => super::TerminalMouseButton::Right,
    }) as i32
}

fn decode_mouse_button(raw: i32) -> WireResult<TerminalMouseButton> {
    Ok(match enum_value(raw, "mouse button")? {
        super::TerminalMouseButton::Unspecified => unreachable!("enum_value rejects 0"),
        super::TerminalMouseButton::Left => TerminalMouseButton::Left,
        super::TerminalMouseButton::Middle => TerminalMouseButton::Middle,
        super::TerminalMouseButton::Right => TerminalMouseButton::Right,
    })
}

fn encode_mouse(event: TerminalMouseEvent) -> super::TerminalMouseEvent {
    use terminal_mouse_event::Event;
    let event = match event {
        TerminalMouseEvent::Button {
            button,
            pressed,
            position,
            modifiers,
        } => Event::Button(TerminalMouseButtonEvent {
            button: encode_mouse_button(button),
            pressed,
            position: Some(encode_mouse_position(position)),
            modifiers: Some(encode_modifiers(modifiers)),
        }),
        TerminalMouseEvent::Motion {
            button,
            position,
            modifiers,
        } => Event::Motion(TerminalMouseMotionEvent {
            button: button.map(encode_mouse_button),
            position: Some(encode_mouse_position(position)),
            modifiers: Some(encode_modifiers(modifiers)),
        }),
        TerminalMouseEvent::Wheel {
            direction,
            amount,
            position,
            modifiers,
        } => Event::Wheel(TerminalMouseWheelEvent {
            direction: match direction {
                TerminalMouseWheel::Up => super::TerminalMouseWheel::Up,
                TerminalMouseWheel::Down => super::TerminalMouseWheel::Down,
                TerminalMouseWheel::Left => super::TerminalMouseWheel::Left,
                TerminalMouseWheel::Right => super::TerminalMouseWheel::Right,
            } as i32,
            amount: amount.into(),
            position: Some(encode_mouse_position(position)),
            modifiers: Some(encode_modifiers(modifiers)),
        }),
    };
    super::TerminalMouseEvent { event: Some(event) }
}

fn decode_mouse(event: super::TerminalMouseEvent) -> WireResult<TerminalMouseEvent> {
    use terminal_mouse_event::Event;
    Ok(match member(event.event)? {
        Event::Button(event) => TerminalMouseEvent::Button {
            button: decode_mouse_button(event.button)?,
            pressed: event.pressed,
            position: decode_mouse_position(event.position)?,
            modifiers: decode_modifiers(event.modifiers),
        },
        Event::Motion(event) => TerminalMouseEvent::Motion {
            button: event.button.map(decode_mouse_button).transpose()?,
            position: decode_mouse_position(event.position)?,
            modifiers: decode_modifiers(event.modifiers),
        },
        Event::Wheel(event) => TerminalMouseEvent::Wheel {
            direction: match enum_value(event.direction, "wheel direction")? {
                super::TerminalMouseWheel::Unspecified => unreachable!("enum_value rejects 0"),
                super::TerminalMouseWheel::Up => TerminalMouseWheel::Up,
                super::TerminalMouseWheel::Down => TerminalMouseWheel::Down,
                super::TerminalMouseWheel::Left => TerminalMouseWheel::Left,
                super::TerminalMouseWheel::Right => TerminalMouseWheel::Right,
            },
            amount: narrow(event.amount, "wheel amount")?,
            position: decode_mouse_position(event.position)?,
            modifiers: decode_modifiers(event.modifiers),
        },
    })
}

fn encode_key(key: &TerminalKey) -> super::TerminalKey {
    use super::TerminalNamedKey as Named;
    use terminal_key::Key;
    let named = |named: Named| Key::Named(named as i32);
    let key = match key {
        TerminalKey::Character(text) => Key::Character(text.clone()),
        TerminalKey::Function(number) => Key::Function((*number).into()),
        TerminalKey::Enter => named(Named::Enter),
        TerminalKey::Tab => named(Named::Tab),
        TerminalKey::BackTab => named(Named::BackTab),
        TerminalKey::Backspace => named(Named::Backspace),
        TerminalKey::Delete => named(Named::Delete),
        TerminalKey::Escape => named(Named::Escape),
        TerminalKey::Up => named(Named::Up),
        TerminalKey::Down => named(Named::Down),
        TerminalKey::Right => named(Named::Right),
        TerminalKey::Left => named(Named::Left),
        TerminalKey::Home => named(Named::Home),
        TerminalKey::End => named(Named::End),
        TerminalKey::PageUp => named(Named::PageUp),
        TerminalKey::PageDown => named(Named::PageDown),
        TerminalKey::Insert => named(Named::Insert),
    };
    super::TerminalKey { key: Some(key) }
}

fn decode_key(key: super::TerminalKey) -> WireResult<TerminalKey> {
    use super::TerminalNamedKey as Named;
    use terminal_key::Key;
    Ok(match member(key.key)? {
        Key::Character(text) => TerminalKey::Character(text),
        Key::Function(number) => TerminalKey::Function(narrow(number, "function key")?),
        Key::Named(raw) => match enum_value(raw, "key")? {
            Named::Unspecified => unreachable!("enum_value rejects 0"),
            Named::Enter => TerminalKey::Enter,
            Named::Tab => TerminalKey::Tab,
            Named::BackTab => TerminalKey::BackTab,
            Named::Backspace => TerminalKey::Backspace,
            Named::Delete => TerminalKey::Delete,
            Named::Escape => TerminalKey::Escape,
            Named::Up => TerminalKey::Up,
            Named::Down => TerminalKey::Down,
            Named::Right => TerminalKey::Right,
            Named::Left => TerminalKey::Left,
            Named::Home => TerminalKey::Home,
            Named::End => TerminalKey::End,
            Named::PageUp => TerminalKey::PageUp,
            Named::PageDown => TerminalKey::PageDown,
            Named::Insert => TerminalKey::Insert,
        },
    })
}

fn encode_scroll(scroll: TerminalScroll) -> super::TerminalScroll {
    use super::TerminalScrollTarget as Target;
    use terminal_scroll::Scroll;
    let target = |target: Target| Scroll::Target(target as i32);
    super::TerminalScroll {
        scroll: Some(match scroll {
            TerminalScroll::Lines(lines) => Scroll::Lines(lines),
            TerminalScroll::PageUp => target(Target::PageUp),
            TerminalScroll::PageDown => target(Target::PageDown),
            TerminalScroll::Top => target(Target::Top),
            TerminalScroll::Bottom => target(Target::Bottom),
        }),
    }
}

fn decode_scroll(scroll: super::TerminalScroll) -> WireResult<TerminalScroll> {
    use super::TerminalScrollTarget as Target;
    use terminal_scroll::Scroll;
    Ok(match member(scroll.scroll)? {
        Scroll::Lines(lines) => TerminalScroll::Lines(lines),
        Scroll::Target(raw) => match enum_value(raw, "scroll target")? {
            Target::Unspecified => unreachable!("enum_value rejects 0"),
            Target::PageUp => TerminalScroll::PageUp,
            Target::PageDown => TerminalScroll::PageDown,
            Target::Top => TerminalScroll::Top,
            Target::Bottom => TerminalScroll::Bottom,
        },
    })
}

impl From<&TerminalCommand> for super::TerminalCommand {
    fn from(command: &TerminalCommand) -> Self {
        use terminal_command::Command;
        let selection = |selection: &Option<crate::TerminalSelection>| TerminalSelectionCommand {
            selection: selection.map(encode_selection),
        };
        let command = match command {
            TerminalCommand::Key { key, modifiers } => Command::Key(TerminalKeyCommand {
                key: Some(encode_key(key)),
                modifiers: Some(encode_modifiers(*modifiers)),
            }),
            TerminalCommand::Text(text) => Command::Text(text.clone()),
            TerminalCommand::Paste(text) => Command::Paste(text.clone()),
            TerminalCommand::Mouse(event) => Command::Mouse(encode_mouse(*event)),
            TerminalCommand::Focus(focused) => Command::Focus(*focused),
            TerminalCommand::Resize(size) => Command::Resize(super::TerminalSize {
                rows: size.rows.into(),
                columns: size.columns.into(),
                cell_width: size.cell_width.into(),
                cell_height: size.cell_height.into(),
            }),
            TerminalCommand::Scroll(scroll) => Command::Scroll(encode_scroll(*scroll)),
            TerminalCommand::Select(target) => Command::Select(selection(target)),
            TerminalCommand::SelectAt {
                position,
                display_offset,
                unit,
            } => Command::SelectAt(TerminalSelectAtCommand {
                position: Some(encode_position(*position)),
                display_offset: *display_offset,
                unit: match unit {
                    TerminalSelectionUnit::Word => super::TerminalSelectionUnit::Word,
                    TerminalSelectionUnit::Line => super::TerminalSelectionUnit::Line,
                } as i32,
            }),
            TerminalCommand::Copy { selection: target } => Command::Copy(selection(target)),
        };
        Self {
            command: Some(command),
        }
    }
}

impl TryFrom<super::TerminalCommand> for TerminalCommand {
    type Error = WireError;

    fn try_from(command: super::TerminalCommand) -> WireResult<Self> {
        use terminal_command::Command;
        Ok(match member(command.command)? {
            Command::Key(key) => Self::Key {
                key: decode_key(required(key.key, "key")?)?,
                modifiers: decode_modifiers(key.modifiers),
            },
            Command::Text(text) => Self::Text(text),
            Command::Paste(text) => Self::Paste(text),
            Command::Mouse(event) => Self::Mouse(decode_mouse(event)?),
            Command::Focus(focused) => Self::Focus(focused),
            Command::Resize(size) => Self::Resize(decode_size(size)?),
            Command::Scroll(scroll) => Self::Scroll(decode_scroll(scroll)?),
            Command::Select(select) => {
                Self::Select(select.selection.map(decode_selection).transpose()?)
            }
            Command::SelectAt(select) => Self::SelectAt {
                position: decode_position(required(select.position, "selection position")?)?,
                display_offset: select.display_offset,
                unit: match enum_value(select.unit, "selection unit")? {
                    super::TerminalSelectionUnit::Unspecified => {
                        unreachable!("enum_value rejects 0")
                    }
                    super::TerminalSelectionUnit::Word => TerminalSelectionUnit::Word,
                    super::TerminalSelectionUnit::Line => TerminalSelectionUnit::Line,
                },
            },
            Command::Copy(copy) => Self::Copy {
                selection: copy.selection.map(decode_selection).transpose()?,
            },
        })
    }
}
