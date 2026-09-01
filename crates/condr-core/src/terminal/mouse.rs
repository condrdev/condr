use super::*;

pub(super) const MAX_MOUSE_WHEEL_STEPS: u16 = 64;
const MAX_UTF8_MOUSE_COORDINATE: u16 = 2014;

pub(super) fn encode_mouse(event: TerminalMouseEvent, modes: TermMode) -> Option<Vec<u8>> {
    if !modes.intersects(TermMode::MOUSE_MODE) {
        return None;
    }

    let (button, release, position, modifiers, repeat) = match event {
        TerminalMouseEvent::Button {
            button,
            pressed,
            position,
            modifiers,
        } => (button_code(button), !pressed, position, modifiers, 1),
        TerminalMouseEvent::Motion {
            button,
            position,
            modifiers,
        } if modes.contains(TermMode::MOUSE_MOTION)
            || (modes.contains(TermMode::MOUSE_DRAG) && button.is_some()) =>
        {
            (
                button.map_or(35, |button| button_code(button) + 32),
                false,
                position,
                modifiers,
                1,
            )
        }
        TerminalMouseEvent::Motion { .. } => return None,
        TerminalMouseEvent::Wheel {
            direction,
            amount,
            position,
            modifiers,
        } => (
            match direction {
                TerminalMouseWheel::Up => 64,
                TerminalMouseWheel::Down => 65,
                TerminalMouseWheel::Left => 66,
                TerminalMouseWheel::Right => 67,
            },
            false,
            position,
            modifiers,
            amount.min(MAX_MOUSE_WHEEL_STEPS),
        ),
    };
    if repeat == 0 {
        return None;
    }

    let report = encode_report(button, release, position, modifiers, modes)?;
    Some(report.repeat(usize::from(repeat)))
}

fn button_code(button: TerminalMouseButton) -> u16 {
    match button {
        TerminalMouseButton::Left => 0,
        TerminalMouseButton::Middle => 1,
        TerminalMouseButton::Right => 2,
    }
}

fn encode_report(
    base_button: u16,
    release: bool,
    position: TerminalMousePosition,
    modifiers: TerminalModifiers,
    modes: TermMode,
) -> Option<Vec<u8>> {
    let mut button = if release && !modes.contains(TermMode::SGR_MOUSE) {
        3
    } else {
        base_button
    };
    button += 4 * u16::from(modifiers.shift);
    button += 8 * u16::from(modifiers.alt);
    button += 16 * u16::from(modifiers.control);

    let column = u32::from(position.column) + 1;
    let row = u32::from(position.row) + 1;
    if modes.contains(TermMode::SGR_MOUSE) {
        return Some(
            format!(
                "\x1b[<{button};{column};{row}{}",
                if release { 'm' } else { 'M' }
            )
            .into_bytes(),
        );
    }

    let mut bytes = Vec::with_capacity(16);
    bytes.extend_from_slice(b"\x1b[M");
    if modes.contains(TermMode::UTF8_MOUSE) {
        if position.row > MAX_UTF8_MOUSE_COORDINATE || position.column > MAX_UTF8_MOUSE_COORDINATE {
            return None;
        }
        push_codepoint(&mut bytes, u32::from(button) + 32)?;
        push_codepoint(&mut bytes, column + 32)?;
        push_codepoint(&mut bytes, row + 32)?;
    } else {
        bytes.push(u8::try_from(button + 32).ok()?);
        bytes.push(u8::try_from(column + 32).ok()?);
        bytes.push(u8::try_from(row + 32).ok()?);
    }
    Some(bytes)
}

fn push_codepoint(bytes: &mut Vec<u8>, value: u32) -> Option<()> {
    let character = char::from_u32(value)?;
    let mut encoded = [0; 4];
    bytes.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position() -> TerminalMousePosition {
        TerminalMousePosition { row: 3, column: 7 }
    }

    #[test]
    fn sgr_mouse_encodes_buttons_motion_wheel_and_modifiers() {
        let modes = TermMode::MOUSE_DRAG | TermMode::SGR_MOUSE;
        let modifiers = TerminalModifiers {
            shift: true,
            control: true,
            ..TerminalModifiers::default()
        };
        assert_eq!(
            encode_mouse(
                TerminalMouseEvent::Button {
                    button: TerminalMouseButton::Left,
                    pressed: true,
                    position: position(),
                    modifiers,
                },
                modes,
            )
            .unwrap(),
            b"\x1b[<20;8;4M"
        );
        assert_eq!(
            encode_mouse(
                TerminalMouseEvent::Button {
                    button: TerminalMouseButton::Left,
                    pressed: false,
                    position: position(),
                    modifiers: TerminalModifiers::default(),
                },
                modes,
            )
            .unwrap(),
            b"\x1b[<0;8;4m"
        );
        assert!(
            encode_mouse(
                TerminalMouseEvent::Motion {
                    button: None,
                    position: position(),
                    modifiers: TerminalModifiers::default(),
                },
                modes,
            )
            .is_none()
        );
        assert_eq!(
            encode_mouse(
                TerminalMouseEvent::Wheel {
                    direction: TerminalMouseWheel::Down,
                    amount: 2,
                    position: position(),
                    modifiers: TerminalModifiers::default(),
                },
                modes,
            )
            .unwrap(),
            b"\x1b[<65;8;4M\x1b[<65;8;4M"
        );
    }

    #[test]
    fn legacy_mouse_uses_release_code_and_utf8_coordinates() {
        let default_modes = TermMode::MOUSE_REPORT_CLICK;
        assert_eq!(
            encode_mouse(
                TerminalMouseEvent::Button {
                    button: TerminalMouseButton::Right,
                    pressed: false,
                    position: TerminalMousePosition { row: 1, column: 2 },
                    modifiers: TerminalModifiers::default(),
                },
                default_modes,
            )
            .unwrap(),
            b"\x1b[M#\x23\x22"
        );

        let utf8_modes = TermMode::MOUSE_MOTION | TermMode::UTF8_MOUSE;
        assert_eq!(
            encode_mouse(
                TerminalMouseEvent::Motion {
                    button: None,
                    position: TerminalMousePosition {
                        row: 130,
                        column: 130,
                    },
                    modifiers: TerminalModifiers::default(),
                },
                utf8_modes,
            )
            .unwrap(),
            vec![b'\x1b', b'[', b'M', b'C', 0xc2, 0xa3, 0xc2, 0xa3]
        );

        let motion_at = |row, column| TerminalMouseEvent::Motion {
            button: None,
            position: TerminalMousePosition { row, column },
            modifiers: TerminalModifiers::default(),
        };
        assert!(
            encode_mouse(
                motion_at(MAX_UTF8_MOUSE_COORDINATE, MAX_UTF8_MOUSE_COORDINATE),
                utf8_modes,
            )
            .is_some()
        );
        for event in [
            motion_at(MAX_UTF8_MOUSE_COORDINATE + 1, 0),
            motion_at(0, MAX_UTF8_MOUSE_COORDINATE + 1),
        ] {
            assert!(encode_mouse(event, utf8_modes).is_none());
        }
        assert!(
            encode_mouse(
                motion_at(MAX_UTF8_MOUSE_COORDINATE + 1, MAX_UTF8_MOUSE_COORDINATE + 1,),
                TermMode::MOUSE_MOTION | TermMode::SGR_MOUSE,
            )
            .is_some()
        );
    }
}
