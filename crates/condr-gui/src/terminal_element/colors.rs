use super::*;

/// Minimum APCA Lc between text and its cell background; Lc 45 is the floor
/// for large fluent text and the common terminal default. Becomes a setting
/// once terminal appearance configuration exists.
const MINIMUM_CONTRAST_LC: f32 = 45.0;

/// The colors every Terminal renders with. Set as a GPUI global by the Appearance
/// settings; absent, the built-in default applies.
#[derive(Clone, PartialEq)]
pub(crate) struct TerminalPalette {
    pub(crate) background: Hsla,
    pub(crate) foreground: Hsla,
    pub(crate) cursor: Hsla,
    /// Text under a block cursor.
    pub(crate) cursor_text: Hsla,
    pub(crate) selection: Hsla,
    /// Text inside the selection; `None` keeps each cell's own color.
    pub(crate) selection_text: Option<Hsla>,
    pub(crate) normal: [Hsla; 8],
    pub(crate) bright: [Hsla; 8],
}

impl Global for TerminalPalette {}

impl Default for TerminalPalette {
    fn default() -> Self {
        // The Server's OSC 4/10/11 replies quote the same palette, so a program that asks
        // and a program that assumes agree.
        let ansi = |index: usize| Hsla::from(rgb(DEFAULT_ANSI_COLORS[index]));
        Self {
            background: rgb(DEFAULT_BACKGROUND_COLOR).into(),
            foreground: rgb(DEFAULT_FOREGROUND_COLOR).into(),
            cursor: rgb(DEFAULT_CURSOR_COLOR).into(),
            cursor_text: rgb(DEFAULT_BACKGROUND_COLOR).into(),
            selection: rgb(0x264f78).into(),
            selection_text: None,
            normal: std::array::from_fn(ansi),
            bright: std::array::from_fn(|index| ansi(index + 8)),
        }
    }
}

/// Box-like characters used as seamless visual connectors: adjusting their
/// color for contrast would break the joins with neighboring backgrounds,
/// so they keep their exact colors.
pub(super) fn is_decorative_character(character: char) -> bool {
    matches!(
        character as u32,
        // Box Drawing, Block Elements and Geometric Shapes.
        0x2500..=0x25FF
        // Legacy Computing sextants.
        | 0x1FB00..=0x1FB3B
        // Powerline separators in the Private Use Area.
        | 0xE0B0..=0xE0BF | 0xE0C0..=0xE0CA | 0xE0CC..=0xE0D7
    )
}

/// Whether the application explicitly picked this color and does not want it
/// adjusted for contrast: 24-bit true color, or a specific entry in the
/// 256-color palette outside the 16 theme-defined ANSI colors.
pub(super) fn is_app_chosen_exact_color(color: TerminalColor) -> bool {
    match color {
        TerminalColor::Rgb { .. } => true,
        TerminalColor::Indexed(index) => index >= 16,
        TerminalColor::Named(_) => false,
    }
}

/// Per-frame memo for the contrast adjustment: a terminal frame holds few
/// distinct color pairs, so a linear scan beats hashing float colors.
#[derive(Default)]
pub(super) struct ContrastMemo {
    pub(super) entries: Vec<(Hsla, Hsla, Hsla)>,
}

impl ContrastMemo {
    pub(super) fn ensure(&mut self, foreground: Hsla, background: Hsla) -> Hsla {
        if let Some((_, _, adjusted)) = self
            .entries
            .iter()
            .find(|(fg, bg, _)| *fg == foreground && *bg == background)
        {
            return *adjusted;
        }
        let adjusted =
            crate::apca::ensure_minimum_contrast(foreground, background, MINIMUM_CONTRAST_LC);
        if self.entries.len() < 256 {
            self.entries.push((foreground, background, adjusted));
        }
        adjusted
    }
}

pub(super) fn terminal_color(
    color: TerminalColor,
    foreground: bool,
    palette: &TerminalPalette,
) -> Hsla {
    match color {
        TerminalColor::Rgb { red, green, blue } => {
            rgb((u32::from(red) << 16) | (u32::from(green) << 8) | u32::from(blue)).into()
        }
        TerminalColor::Indexed(index) => indexed_color(index, palette),
        TerminalColor::Named(256 | 267) => palette.foreground,
        TerminalColor::Named(257) => palette.background,
        TerminalColor::Named(258) => palette.cursor,
        TerminalColor::Named(index @ 259..=266) => {
            palette.normal[usize::from(index - 259)].opacity(0.65)
        }
        TerminalColor::Named(268) => palette.foreground.opacity(0.65),
        TerminalColor::Named(index @ 0..=15) => indexed_color(index as u8, palette),
        TerminalColor::Named(_) if foreground => palette.foreground,
        TerminalColor::Named(_) => palette.background,
    }
}

pub(super) fn indexed_color(index: u8, palette: &TerminalPalette) -> Hsla {
    match index {
        0..=7 => palette.normal[usize::from(index)],
        8..=15 => palette.bright[usize::from(index - 8)],
        _ => rgb(default_indexed_color(index)).into(),
    }
}
