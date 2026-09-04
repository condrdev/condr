use super::*;

pub(super) fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        // Only ESC is stripped so pasted text cannot forge the closing marker.
        let text = text.replace('\x1b', "");
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

/// Encodes a key press for the live terminal modes: the kitty keyboard protocol when
/// the application negotiated it, otherwise the legacy xterm sequences.
pub(super) fn encode_key_in_mode(
    key: &TerminalKey,
    modifiers: TerminalModifiers,
    modes: TermMode,
) -> io::Result<Vec<u8>> {
    if let Some(bytes) = encode_kitty_key(key, modifiers, modes) {
        return Ok(bytes);
    }
    encode_key(key, modifiers, modes.contains(TermMode::APP_CURSOR))
}

/// Kitty keyboard protocol, press events only, aligned with herdr: plain text and
/// unmodified Enter/Tab/Backspace stay legacy unless every key is to be reported, and
/// keys with well-known xterm modified forms keep those. Only Escape follows the spec
/// rather than herdr and becomes `CSI 27 u` under disambiguation.
fn encode_kitty_key(
    key: &TerminalKey,
    modifiers: TerminalModifiers,
    modes: TermMode,
) -> Option<Vec<u8>> {
    if !modes.intersects(TermMode::KITTY_KEYBOARD_PROTOCOL) {
        return None;
    }
    let report_all = modes.contains(TermMode::REPORT_ALL_KEYS_AS_ESC);
    let event_types = modes.contains(TermMode::REPORT_EVENT_TYPES);
    let text_only = !modifiers.control && !modifiers.alt && !modifiers.platform;
    let modified = !text_only || modifiers.shift;
    let escape_only = !report_all && !event_types;
    match key {
        TerminalKey::Character(_) if text_only && escape_only => return None,
        TerminalKey::Enter | TerminalKey::Tab | TerminalKey::Backspace
            if !modified && !report_all =>
        {
            return None;
        }
        TerminalKey::Up
        | TerminalKey::Down
        | TerminalKey::Right
        | TerminalKey::Left
        | TerminalKey::Home
        | TerminalKey::End
        | TerminalKey::Insert
        | TerminalKey::Delete
        | TerminalKey::PageUp
        | TerminalKey::PageDown
        | TerminalKey::Function(_)
            if escape_only =>
        {
            return None;
        }
        TerminalKey::Function(number) if !(1..=20).contains(number) => return None,
        _ => {}
    }

    let shift = modifiers.shift || matches!(key, TerminalKey::BackTab);
    let modifier = 1
        + u8::from(shift)
        + 2 * u8::from(modifiers.alt)
        + 4 * u8::from(modifiers.control)
        + 8 * u8::from(modifiers.platform);
    let mods = kitty_modifier_field(modifier, event_types);
    let legacy_form = |prefix: u8, final_byte: char| format!("\x1b[{prefix};{mods}{final_byte}");
    let csi_u = |codepoint: u32| format!("\x1b[{codepoint}{}u", modifier_suffix(&mods));
    let sequence = match key {
        TerminalKey::Up => legacy_form(1, 'A'),
        TerminalKey::Down => legacy_form(1, 'B'),
        TerminalKey::Right => legacy_form(1, 'C'),
        TerminalKey::Left => legacy_form(1, 'D'),
        TerminalKey::Home => legacy_form(1, 'H'),
        TerminalKey::End => legacy_form(1, 'F'),
        TerminalKey::Insert => legacy_form(2, '~'),
        TerminalKey::Delete => legacy_form(3, '~'),
        TerminalKey::PageUp => legacy_form(5, '~'),
        TerminalKey::PageDown => legacy_form(6, '~'),
        TerminalKey::Function(1) => legacy_form(1, 'P'),
        TerminalKey::Function(2) => legacy_form(1, 'Q'),
        TerminalKey::Function(3) => legacy_form(13, '~'),
        TerminalKey::Function(4) => legacy_form(1, 'S'),
        TerminalKey::Function(number) => {
            const TILDE_CODES: [u8; 16] = [
                15, 17, 18, 19, 20, 21, 23, 24, 25, 26, 28, 29, 31, 32, 33, 34,
            ];
            legacy_form(TILDE_CODES[usize::from(*number) - 5], '~')
        }
        TerminalKey::Enter => csi_u(13),
        TerminalKey::Tab | TerminalKey::BackTab => csi_u(9),
        TerminalKey::Backspace => csi_u(127),
        TerminalKey::Escape => csi_u(27),
        TerminalKey::Character(text) => {
            let mut chars = text.chars();
            let ch = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            kitty_character(ch, modifiers.shift, &mods, text_only, modes)
        }
    };
    Some(sequence.into_bytes())
}

/// `CSI unicode[:shifted] ; mods [; text] u` for one produced character. Shift on an
/// ASCII letter reports the lowercase base key with the uppercase as the alternate.
fn kitty_character(ch: char, shift: bool, mods: &str, text_only: bool, modes: TermMode) -> String {
    let shifted_letter = shift && ch.is_ascii_uppercase();
    let base = if shifted_letter {
        ch.to_ascii_lowercase()
    } else {
        ch
    };
    let mut sequence = format!("\x1b[{}", u32::from(base));
    if shifted_letter && modes.contains(TermMode::REPORT_ALTERNATE_KEYS) {
        sequence.push_str(&format!(":{}", u32::from(ch)));
    }
    if text_only && !ch.is_control() && modes.contains(TermMode::REPORT_ASSOCIATED_TEXT) {
        let mods = if mods.is_empty() { "1" } else { mods };
        sequence.push_str(&format!(";{mods};{}", u32::from(ch)));
    } else {
        sequence.push_str(&modifier_suffix(mods));
    }
    sequence.push('u');
    sequence
}

/// The kitty modifier field; empty when it would be the default `1` without an event
/// type, which the protocol lets the terminal omit.
fn kitty_modifier_field(modifier: u8, event_types: bool) -> String {
    match (modifier, event_types) {
        (1, false) => String::new(),
        (_, true) => format!("{modifier}:1"),
        (_, false) => modifier.to_string(),
    }
}

fn modifier_suffix(mods: &str) -> String {
    if mods.is_empty() {
        String::new()
    } else {
        format!(";{mods}")
    }
}

/// Typed text while every key must be reported as an escape code. Single characters
/// become kitty key reports (uppercase letters as shifted keys); longer commits, such as
/// IME input, stay plain text.
pub(super) fn encode_text_in_mode(text: &str, modes: TermMode) -> Vec<u8> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) if modes.contains(TermMode::REPORT_ALL_KEYS_AS_ESC) => {
            let shift = ch.is_ascii_uppercase();
            let mods = kitty_modifier_field(
                if shift { 2 } else { 1 },
                modes.contains(TermMode::REPORT_EVENT_TYPES),
            );
            kitty_character(ch, shift, &mods, true, modes).into_bytes()
        }
        _ => text.as_bytes().to_vec(),
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
        // Shift+Enter sends LF (Ctrl+J), which agent CLIs treat as newline.
        TerminalKey::Enter if modifiers.shift => "\n".into(),
        TerminalKey::Enter => "\r".into(),
        TerminalKey::Tab if modifiers.shift => "\x1b[Z".into(),
        TerminalKey::Tab => "\t".into(),
        TerminalKey::BackTab => "\x1b[Z".into(),
        TerminalKey::Backspace if modifiers.control => "\x08".into(),
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
        b'@' | b' ' | b'2' => Some(0),
        b'a'..=b'z' => Some(byte - b'a' + 1),
        b'[' | b'3' => Some(27),
        b'\\' | b'4' => Some(28),
        b']' | b'5' => Some(29),
        b'^' | b'6' => Some(30),
        b'_' | b'/' | b'7' | b'-' => Some(31),
        b'?' => Some(127),
        _ => None,
    }
}

fn modifier_code(modifiers: TerminalModifiers) -> u8 {
    1 + u8::from(modifiers.shift) + 2 * u8::from(modifiers.alt) + 4 * u8::from(modifiers.control)
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
        13 => tilde_sequence(25, modifier),
        14 => tilde_sequence(26, modifier),
        15 => tilde_sequence(28, modifier),
        16 => tilde_sequence(29, modifier),
        17 => tilde_sequence(31, modifier),
        18 => tilde_sequence(32, modifier),
        19 => tilde_sequence(33, modifier),
        20 => tilde_sequence(34, modifier),
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "terminal function key must be F1 through F20",
            ));
        }
    };
    Ok(sequence)
}
