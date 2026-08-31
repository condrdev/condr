use super::*;

pub(super) fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        let text = text.replace(['\x1b', '\x03'], "");
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

pub(super) fn encode_key(
    key: &TerminalKey,
    modifiers: TerminalModifiers,
    application_cursor: bool,
) -> io::Result<Vec<u8>> {
    if let TerminalKey::Character(text) = key {
        let mut bytes = if modifiers.control {
            control_character(text).map_or_else(|| text.as_bytes().to_vec(), |byte| vec![byte])
        } else {
            text.as_bytes().to_vec()
        };
        if modifiers.alt {
            bytes.insert(0, b'\x1b');
        }
        return Ok(bytes);
    }

    let modifier = modifier_code(modifiers);
    let sequence = match key {
        TerminalKey::Enter => "\r".into(),
        TerminalKey::Tab if modifiers.shift => "\x1b[Z".into(),
        TerminalKey::Tab => "\t".into(),
        TerminalKey::BackTab => "\x1b[Z".into(),
        TerminalKey::Backspace => "\x7f".into(),
        TerminalKey::Escape => "\x1b".into(),
        TerminalKey::Up => cursor_sequence('A', modifier, application_cursor),
        TerminalKey::Down => cursor_sequence('B', modifier, application_cursor),
        TerminalKey::Right => cursor_sequence('C', modifier, application_cursor),
        TerminalKey::Left => cursor_sequence('D', modifier, application_cursor),
        TerminalKey::Home => cursor_sequence('H', modifier, application_cursor),
        TerminalKey::End => cursor_sequence('F', modifier, application_cursor),
        TerminalKey::Insert => tilde_sequence(2, modifier),
        TerminalKey::Delete => tilde_sequence(3, modifier),
        TerminalKey::PageUp => tilde_sequence(5, modifier),
        TerminalKey::PageDown => tilde_sequence(6, modifier),
        TerminalKey::Function(number) => function_sequence(*number, modifier)?,
        TerminalKey::Character(_) => unreachable!(),
    };

    let mut bytes = sequence.into_bytes();
    if modifiers.alt
        && matches!(
            key,
            TerminalKey::Enter
                | TerminalKey::Tab
                | TerminalKey::BackTab
                | TerminalKey::Backspace
                | TerminalKey::Escape
        )
    {
        bytes.insert(0, b'\x1b');
    }
    Ok(bytes)
}

fn control_character(text: &str) -> Option<u8> {
    let byte = text.as_bytes().first()?.to_ascii_lowercase();
    match byte {
        b'@' | b' ' => Some(0),
        b'a'..=b'z' => Some(byte - b'a' + 1),
        b'[' => Some(27),
        b'\\' => Some(28),
        b']' => Some(29),
        b'^' => Some(30),
        b'_' => Some(31),
        b'?' => Some(127),
        _ => None,
    }
}

fn modifier_code(modifiers: TerminalModifiers) -> u8 {
    1 + u8::from(modifiers.shift)
        + 2 * u8::from(modifiers.alt)
        + 4 * u8::from(modifiers.control)
        + 8 * u8::from(modifiers.platform)
}

fn cursor_sequence(final_byte: char, modifier: u8, application_cursor: bool) -> String {
    if modifier > 1 {
        format!("\x1b[1;{modifier}{final_byte}")
    } else if application_cursor {
        format!("\x1bO{final_byte}")
    } else {
        format!("\x1b[{final_byte}")
    }
}

fn tilde_sequence(code: u8, modifier: u8) -> String {
    if modifier > 1 {
        format!("\x1b[{code};{modifier}~")
    } else {
        format!("\x1b[{code}~")
    }
}

fn function_sequence(number: u8, modifier: u8) -> io::Result<String> {
    let sequence = match number {
        1..=4 if modifier > 1 => {
            let final_byte = char::from(b'P' + number - 1);
            format!("\x1b[1;{modifier}{final_byte}")
        }
        1..=4 => {
            let final_byte = char::from(b'P' + number - 1);
            format!("\x1bO{final_byte}")
        }
        5 => tilde_sequence(15, modifier),
        6 => tilde_sequence(17, modifier),
        7 => tilde_sequence(18, modifier),
        8 => tilde_sequence(19, modifier),
        9 => tilde_sequence(20, modifier),
        10 => tilde_sequence(21, modifier),
        11 => tilde_sequence(23, modifier),
        12 => tilde_sequence(24, modifier),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "terminal function key must be F1 through F12",
            ));
        }
    };
    Ok(sequence)
}
