//! libghostty-vt, Ghostty's terminal core, for the Server's terminals (ADR 0051): the
//! bindings script/update-libghostty-vt.sh generates and a thin safe layer over the part
//! Condr calls. Every `unsafe` the VT needs lives in this crate.
#![allow(unsafe_code)]

#[allow(
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    dead_code,
    clippy::all
)]
mod bindings;

use bindings as sys;
use std::{fmt, mem::size_of, ptr, slice};

/// What a libghostty-vt call that did not succeed returned.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error(i32);

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let reason = match sys::GhosttyResult(self.0) {
            sys::GhosttyResult::GHOSTTY_OUT_OF_MEMORY => "out of memory",
            sys::GhosttyResult::GHOSTTY_INVALID_VALUE => "invalid value",
            sys::GhosttyResult::GHOSTTY_OUT_OF_SPACE => "buffer too small",
            sys::GhosttyResult::GHOSTTY_NO_VALUE => "no value",
            sys::GhosttyResult::GHOSTTY_IO_ERROR => "I/O error",
            sys::GhosttyResult::GHOSTTY_LIMIT_EXCEEDED => "limit exceeded",
            sys::GhosttyResult::GHOSTTY_REJECTED => "rejected",
            _ => "unknown result",
        };
        write!(formatter, "libghostty-vt: {reason} ({})", self.0)
    }
}

impl std::error::Error for Error {}

fn check(result: sys::GhosttyResult) -> Result<(), Error> {
    if result == sys::GhosttyResult::GHOSTTY_SUCCESS {
        Ok(())
    } else {
        Err(Error(result.0))
    }
}

/// One terminal: its screens, scrollback and modes, fed the bytes its program writes.
pub struct Terminal(sys::GhosttyTerminal);

impl Terminal {
    pub fn new(cols: u16, rows: u16) -> Result<Self, Error> {
        let mut terminal = ptr::null_mut();
        // SAFETY: a null allocator is the library's default; `terminal` is only read
        // once the call succeeded.
        check(unsafe { sys::ghostty_terminal_new(ptr::null(), &mut terminal, cols, rows) })?;
        Ok(Self(terminal))
    }

    /// Feeds output the program wrote to its PTY.
    pub fn write(&mut self, bytes: &[u8]) {
        // SAFETY: the terminal is live, `&mut self` serializes calls on it, and the
        // pointer and length describe `bytes`.
        unsafe { sys::ghostty_terminal_vt_write(self.0, bytes.as_ptr(), bytes.len()) }
    }

    /// The screen as plain text, soft-wrapped lines joined and trailing blanks trimmed.
    pub fn plain_text(&self) -> Result<String, Error> {
        let options = sys::GhosttyFormatterTerminalOptions {
            size: size_of::<sys::GhosttyFormatterTerminalOptions>(),
            emit: sys::GhosttyFormatterFormat::GHOSTTY_FORMATTER_FORMAT_PLAIN,
            unwrap: true,
            trim: true,
            extra: sys::GhosttyFormatterTerminalExtra {
                size: size_of::<sys::GhosttyFormatterTerminalExtra>(),
                palette: false,
                modes: false,
                scrolling_region: false,
                tabstops: false,
                pwd: false,
                keyboard: false,
                screen: sys::GhosttyFormatterScreenExtra {
                    size: size_of::<sys::GhosttyFormatterScreenExtra>(),
                    cursor: false,
                    style: false,
                    hyperlink: false,
                    protection: false,
                    kitty_keyboard: false,
                    charsets: false,
                },
            },
            selection: ptr::null(),
        };
        let mut formatter = ptr::null_mut();
        // SAFETY: the terminal outlives the formatter, which is freed below.
        check(unsafe {
            sys::ghostty_formatter_terminal_new(ptr::null(), &mut formatter, self.0, options)
        })?;
        let (mut text, mut len) = (ptr::null_mut(), 0);
        // SAFETY: a live formatter; `text` and `len` are only read once the call succeeded.
        let result = unsafe {
            sys::ghostty_formatter_format_alloc(formatter, ptr::null(), &mut text, &mut len)
        };
        // SAFETY: created above and not used again.
        unsafe { sys::ghostty_formatter_free(formatter) };
        check(result)?;
        if text.is_null() {
            return Ok(String::new());
        }
        // SAFETY: the library allocated `len` bytes at `text` with the default allocator,
        // and they are freed with it only after being copied.
        let plain =
            String::from_utf8_lossy(unsafe { slice::from_raw_parts(text, len) }).into_owned();
        unsafe { sys::ghostty_free(ptr::null(), text, len) };
        Ok(plain)
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // SAFETY: created by `new` and freed only here.
        unsafe { sys::ghostty_terminal_free(self.0) }
    }
}

#[cfg(test)]
mod tests {
    use super::Terminal;

    #[test]
    fn a_terminal_shows_what_its_program_wrote() {
        let mut terminal = Terminal::new(20, 5).unwrap();
        terminal.write(b"hello\r\n\x1b[1mworld\x1b[0m");
        assert_eq!(terminal.plain_text().unwrap(), "hello\nworld");
    }
}
