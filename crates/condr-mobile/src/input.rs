//! What the phone's touches and keys send, the same `TerminalCommand`s the desktop sends for
//! the same keys, clicks and wheel motion (ADR 0041). The Server decides what they mean in
//! the Pane: a program that tracks the mouse gets the click and the wheel, an alternate
//! screen that asks turns the wheel into arrow keys, and otherwise the wheel scrolls.

use crate::views::Key;
use condr_core::{
    TerminalCommand, TerminalKey, TerminalKeyEventKind, TerminalModifiers, TerminalMouseButton,
    TerminalMouseEvent, TerminalMousePosition, TerminalMouseWheel, TerminalPosition,
    TerminalSelection, TerminalSide,
};

/// The longest run of wheel steps one swipe event sends; the Server caps it again.
const MAX_WHEEL_STEPS: u16 = 64;

/// A key from the row above the soft keyboard, with its sticky Ctrl.
pub(crate) fn key(key: Key, control: bool) -> TerminalCommand {
    let key = match key {
        Key::Enter => TerminalKey::Enter,
        Key::Escape => TerminalKey::Escape,
        Key::Tab => TerminalKey::Tab,
        Key::BackTab => TerminalKey::BackTab,
        Key::Backspace => TerminalKey::Backspace,
        Key::Delete => TerminalKey::Delete,
        Key::Up => TerminalKey::Up,
        Key::Down => TerminalKey::Down,
        Key::Left => TerminalKey::Left,
        Key::Right => TerminalKey::Right,
        Key::Home => TerminalKey::Home,
        Key::End => TerminalKey::End,
        Key::PageUp => TerminalKey::PageUp,
        Key::PageDown => TerminalKey::PageDown,
        Key::Character { text } => TerminalKey::Character(text),
    };
    TerminalCommand::Key {
        key,
        modifiers: TerminalModifiers {
            control,
            ..TerminalModifiers::default()
        },
        kind: TerminalKeyEventKind::Press,
    }
}

/// A swipe of `lines` over the cell at `row`, `column`, positive towards what came before:
/// wheel motion, as the GUI sends a trackpad's. None for no motion.
pub(crate) fn wheel(row: u16, column: u16, lines: i32) -> Option<TerminalCommand> {
    let amount = u16::try_from(lines.unsigned_abs())
        .unwrap_or(u16::MAX)
        .min(MAX_WHEEL_STEPS);
    (amount > 0).then(|| {
        TerminalCommand::Mouse(TerminalMouseEvent::Wheel {
            direction: if lines > 0 {
                TerminalMouseWheel::Up
            } else {
                TerminalMouseWheel::Down
            },
            amount,
            position: TerminalMousePosition { row, column },
            modifiers: TerminalModifiers::default(),
        })
    })
}

/// A tap where the program tracks the mouse: the left button pressed and let go.
pub(crate) fn click(row: u16, column: u16) -> [TerminalCommand; 2] {
    let button = |pressed| {
        TerminalCommand::Mouse(TerminalMouseEvent::Button {
            button: TerminalMouseButton::Left,
            pressed,
            position: TerminalMousePosition { row, column },
            modifiers: TerminalModifiers::default(),
        })
    };
    [button(true), button(false)]
}

/// The cells from `from` to `to`, both included, whichever comes first, on the viewport
/// scrolled `display_offset` lines back.
pub(crate) fn range(from: (u16, u16), to: (u16, u16), display_offset: u32) -> TerminalSelection {
    let (first, last) = if to >= from {
        (TerminalSide::Left, TerminalSide::Right)
    } else {
        (TerminalSide::Right, TerminalSide::Left)
    };
    TerminalSelection {
        start: TerminalPosition {
            row: from.0,
            column: from.1,
            side: first,
        },
        end: TerminalPosition {
            row: to.0,
            column: to.1,
            side: last,
        },
        display_offset,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_carry_the_sticky_ctrl_as_the_gui_sends_them() {
        assert_eq!(
            key(Key::Character { text: "c".into() }, true),
            TerminalCommand::Key {
                key: TerminalKey::Character("c".into()),
                modifiers: TerminalModifiers {
                    control: true,
                    ..TerminalModifiers::default()
                },
                kind: TerminalKeyEventKind::Press,
            }
        );
        assert!(matches!(
            key(Key::BackTab, false),
            TerminalCommand::Key {
                key: TerminalKey::BackTab,
                modifiers: TerminalModifiers { control: false, .. },
                ..
            }
        ));
    }

    #[test]
    fn a_swipe_is_wheel_motion_at_the_finger() {
        assert_eq!(
            wheel(3, 7, 2),
            Some(TerminalCommand::Mouse(TerminalMouseEvent::Wheel {
                direction: TerminalMouseWheel::Up,
                amount: 2,
                position: TerminalMousePosition { row: 3, column: 7 },
                modifiers: TerminalModifiers::default(),
            }))
        );
        assert!(matches!(
            wheel(0, 0, -900),
            Some(TerminalCommand::Mouse(TerminalMouseEvent::Wheel {
                direction: TerminalMouseWheel::Down,
                amount: MAX_WHEEL_STEPS,
                ..
            }))
        ));
        assert_eq!(wheel(0, 0, 0), None);
    }

    #[test]
    fn a_tap_presses_and_lets_go_where_it_landed() {
        let [press, release] = click(4, 9);
        assert!(matches!(
            press,
            TerminalCommand::Mouse(TerminalMouseEvent::Button {
                button: TerminalMouseButton::Left,
                pressed: true,
                position: TerminalMousePosition { row: 4, column: 9 },
                ..
            })
        ));
        assert!(matches!(
            release,
            TerminalCommand::Mouse(TerminalMouseEvent::Button { pressed: false, .. })
        ));
    }

    #[test]
    fn a_drag_selects_both_ends_in_either_direction() {
        let forward = range((1, 2), (3, 0), 5);
        assert_eq!(
            (forward.start.side, forward.end.side),
            (TerminalSide::Left, TerminalSide::Right)
        );
        assert_eq!(forward.display_offset, 5);
        let back = range((3, 0), (1, 2), 0);
        assert_eq!(
            (back.start.side, back.end.side),
            (TerminalSide::Right, TerminalSide::Left)
        );
        assert_eq!((back.start.row, back.end.row), (3, 1));
    }
}
