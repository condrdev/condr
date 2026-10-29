//! Key encoding: libghostty-vt turns a key event into what the program reads, for the
//! terminal's live modes (legacy xterm, modifyOtherKeys, kitty keyboard protocol).

use super::{Error, Terminal, check, sys};
use std::ptr;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    /// Cmd on macOS, the Windows key elsewhere.
    pub logo: bool,
}

impl Mods {
    fn bits(self) -> sys::GhosttyMods {
        u16::from(self.shift)
            | u16::from(self.ctrl) << 1
            | u16::from(self.alt) << 2
            | u16::from(self.logo) << 3
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KeyAction {
    Press,
    Repeat,
    Release,
}

/// A physical key, named by where it sits on a US layout.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PhysicalKey {
    /// A letter, digit or punctuation key, by the character it types unshifted.
    Char(char),
    Space,
    Enter,
    Tab,
    Backspace,
    Delete,
    Escape,
    Up,
    Down,
    Right,
    Left,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Function(u8),
    Unidentified,
}

impl PhysicalKey {
    fn to_sys(self) -> sys::GhosttyKey {
        use sys::GhosttyKey as Key;
        let offset = |first: Key, by: u32| Key(first.0 + by as i32);
        match self {
            Self::Char(c @ 'a'..='z') => offset(Key::GHOSTTY_KEY_A, c as u32 - 'a' as u32),
            Self::Char(c @ '0'..='9') => offset(Key::GHOSTTY_KEY_DIGIT_0, c as u32 - '0' as u32),
            Self::Char('`') => Key::GHOSTTY_KEY_BACKQUOTE,
            Self::Char('\\') => Key::GHOSTTY_KEY_BACKSLASH,
            Self::Char('[') => Key::GHOSTTY_KEY_BRACKET_LEFT,
            Self::Char(']') => Key::GHOSTTY_KEY_BRACKET_RIGHT,
            Self::Char(',') => Key::GHOSTTY_KEY_COMMA,
            Self::Char('=') => Key::GHOSTTY_KEY_EQUAL,
            Self::Char('-') => Key::GHOSTTY_KEY_MINUS,
            Self::Char('.') => Key::GHOSTTY_KEY_PERIOD,
            Self::Char('\'') => Key::GHOSTTY_KEY_QUOTE,
            Self::Char(';') => Key::GHOSTTY_KEY_SEMICOLON,
            Self::Char('/') => Key::GHOSTTY_KEY_SLASH,
            Self::Char(' ') | Self::Space => Key::GHOSTTY_KEY_SPACE,
            Self::Char(_) | Self::Unidentified => Key::GHOSTTY_KEY_UNIDENTIFIED,
            Self::Enter => Key::GHOSTTY_KEY_ENTER,
            Self::Tab => Key::GHOSTTY_KEY_TAB,
            Self::Backspace => Key::GHOSTTY_KEY_BACKSPACE,
            Self::Delete => Key::GHOSTTY_KEY_DELETE,
            Self::Escape => Key::GHOSTTY_KEY_ESCAPE,
            Self::Up => Key::GHOSTTY_KEY_ARROW_UP,
            Self::Down => Key::GHOSTTY_KEY_ARROW_DOWN,
            Self::Right => Key::GHOSTTY_KEY_ARROW_RIGHT,
            Self::Left => Key::GHOSTTY_KEY_ARROW_LEFT,
            Self::Home => Key::GHOSTTY_KEY_HOME,
            Self::End => Key::GHOSTTY_KEY_END,
            Self::PageUp => Key::GHOSTTY_KEY_PAGE_UP,
            Self::PageDown => Key::GHOSTTY_KEY_PAGE_DOWN,
            Self::Insert => Key::GHOSTTY_KEY_INSERT,
            Self::Function(n @ 1..=25) => offset(Key::GHOSTTY_KEY_F1, u32::from(n) - 1),
            Self::Function(_) => Key::GHOSTTY_KEY_UNIDENTIFIED,
        }
    }
}

/// One key event as a keyboard would produce it.
#[derive(Clone, Copy, Debug)]
pub struct KeyEvent<'a> {
    pub key: PhysicalKey,
    pub action: KeyAction,
    pub mods: Mods,
    /// Modifiers the layout used to produce `text`, such as Shift for `A`.
    pub consumed: Mods,
    /// The text the key types, empty for none.
    pub text: &'a str,
    /// The character the key types without modifiers.
    pub unshifted: Option<char>,
}

pub struct KeyEncoder {
    raw: sys::GhosttyKeyEncoder,
    event: sys::GhosttyKeyEvent,
}

// SAFETY: as for `Terminal`: used only through `&mut self`, beside its terminal under
// one mutex.
unsafe impl Send for KeyEncoder {}

impl KeyEncoder {
    pub fn new() -> Result<Self, Error> {
        let mut encoder = Self {
            raw: ptr::null_mut(),
            event: ptr::null_mut(),
        };
        // SAFETY: default allocator; a handle is set only by a call that succeeded, and
        // `Drop` frees whichever were created.
        unsafe {
            check(sys::ghostty_key_encoder_new(ptr::null(), &mut encoder.raw))?;
            check(sys::ghostty_key_event_new(ptr::null(), &mut encoder.event))?;
        }
        Ok(encoder)
    }

    /// What `event` sends to the program, for `terminal`'s current modes. `kitty_flags`
    /// stands in for the flags the program pushed when given.
    pub fn encode(
        &mut self,
        terminal: &Terminal,
        event: &KeyEvent<'_>,
        kitty_flags: Option<u8>,
    ) -> Vec<u8> {
        use sys::GhosttyKeyEncoderOption as Opt;
        let action = match event.action {
            KeyAction::Press => sys::GhosttyKeyAction::GHOSTTY_KEY_ACTION_PRESS,
            KeyAction::Repeat => sys::GhosttyKeyAction::GHOSTTY_KEY_ACTION_REPEAT,
            KeyAction::Release => sys::GhosttyKeyAction::GHOSTTY_KEY_ACTION_RELEASE,
        };
        // Option is Alt whenever the Client says Alt: it already let Option compose text
        // as typing when that was meant (macOS resets this with every terminal read).
        let option_as_alt = sys::GhosttyOptionAsAlt::GHOSTTY_OPTION_AS_ALT_TRUE;
        // SAFETY: live handles, values of the types each option names, and `text`
        // outliving the encode call that reads it.
        unsafe {
            sys::ghostty_key_encoder_setopt_from_terminal(self.raw, terminal.raw);
            sys::ghostty_key_encoder_setopt(
                self.raw,
                Opt::GHOSTTY_KEY_ENCODER_OPT_MACOS_OPTION_AS_ALT,
                (&raw const option_as_alt).cast(),
            );
            if let Some(flags) = kitty_flags {
                sys::ghostty_key_encoder_setopt(
                    self.raw,
                    Opt::GHOSTTY_KEY_ENCODER_OPT_KITTY_FLAGS,
                    (&raw const flags).cast(),
                );
            }
            sys::ghostty_key_event_set_action(self.event, action);
            sys::ghostty_key_event_set_key(self.event, event.key.to_sys());
            sys::ghostty_key_event_set_mods(self.event, event.mods.bits());
            sys::ghostty_key_event_set_consumed_mods(self.event, event.consumed.bits());
            sys::ghostty_key_event_set_utf8(
                self.event,
                event.text.as_ptr().cast(),
                event.text.len(),
            );
            sys::ghostty_key_event_set_unshifted_codepoint(
                self.event,
                event.unshifted.map_or(0, u32::from),
            );
        }
        let mut out = vec![0u8; 64];
        loop {
            let mut len = 0;
            // SAFETY: the buffer is as long as said; `len` is read after the call.
            let result = unsafe {
                sys::ghostty_key_encoder_encode(
                    self.raw,
                    self.event,
                    out.as_mut_ptr().cast(),
                    out.len(),
                    &mut len,
                )
            };
            if result == sys::GhosttyResult::GHOSTTY_OUT_OF_SPACE && len > out.len() {
                out.resize(len, 0);
                continue;
            }
            out.truncate(if check(result).is_ok() { len } else { 0 });
            return out;
        }
    }
}

impl Drop for KeyEncoder {
    fn drop(&mut self) {
        // SAFETY: created in `new` (a null handle is skipped) and freed only here.
        unsafe {
            if !self.event.is_null() {
                sys::ghostty_key_event_free(self.event);
            }
            if !self.raw.is_null() {
                sys::ghostty_key_encoder_free(self.raw);
            }
        }
    }
}
