//! Built-in terminal color schemes, vendored from iTerm2-Color-Schemes in Alacritty's
//! TOML layout. Only the chosen scheme is parsed; the names come for free.

use gpui_kit::{Hsla, rgb};
use serde::Deserialize;

use crate::terminal_element::TerminalPalette;

mod built_in {
    include!(concat!(env!("OUT_DIR"), "/color_schemes.rs"));
}

pub(crate) fn names() -> impl Iterator<Item = &'static str> {
    built_in::BUILT_IN.iter().map(|(name, _)| *name)
}

/// The palette for a built-in scheme name, or `None` when no scheme has that name.
pub(crate) fn palette(name: &str) -> Option<TerminalPalette> {
    let (_, toml) = built_in::BUILT_IN
        .iter()
        .find(|(candidate, _)| *candidate == name)?;
    parse(toml).ok()
}

#[derive(Deserialize)]
struct Scheme {
    colors: Colors,
}

#[derive(Deserialize)]
struct Colors {
    primary: Primary,
    normal: Ansi,
    bright: Ansi,
    cursor: Cursor,
    selection: Selection,
}

#[derive(Deserialize)]
struct Primary {
    background: Color,
    foreground: Color,
}

#[derive(Deserialize)]
struct Cursor {
    cursor: Color,
    text: Option<Color>,
}

#[derive(Deserialize)]
struct Selection {
    background: Color,
    text: Option<Color>,
}

#[derive(Deserialize)]
struct Ansi {
    black: Color,
    red: Color,
    green: Color,
    yellow: Color,
    blue: Color,
    magenta: Color,
    cyan: Color,
    white: Color,
}

impl Ansi {
    fn into_array(self) -> [Hsla; 8] {
        [
            self.black.0,
            self.red.0,
            self.green.0,
            self.yellow.0,
            self.blue.0,
            self.magenta.0,
            self.cyan.0,
            self.white.0,
        ]
    }
}

/// `#rrggbb` or `0xrrggbb`, the two spellings Alacritty accepts.
#[derive(Deserialize)]
#[serde(try_from = "String")]
struct Color(Hsla);

impl TryFrom<String> for Color {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        let digits = text
            .strip_prefix('#')
            .or_else(|| text.strip_prefix("0x"))
            .filter(|digits| digits.len() == 6)
            .ok_or_else(|| format!("{text:?} is not a #rrggbb color"))?;
        u32::from_str_radix(digits, 16)
            .map(|value| Self(rgb(value).into()))
            .map_err(|error| format!("{text:?}: {error}"))
    }
}

fn parse(toml: &str) -> Result<TerminalPalette, toml::de::Error> {
    let Scheme { colors } = toml::from_str(toml)?;
    let background = colors.primary.background.0;
    Ok(TerminalPalette {
        background,
        foreground: colors.primary.foreground.0,
        cursor: colors.cursor.cursor.0,
        cursor_text: colors.cursor.text.map_or(background, |color| color.0),
        selection: colors.selection.background.0,
        selection_text: colors.selection.text.map(|color| color.0),
        normal: colors.normal.into_array(),
        bright: colors.bright.into_array(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_built_in_scheme_parses() {
        assert!(names().count() > 100, "the vendored schemes are missing");
        for (name, toml) in built_in::BUILT_IN {
            parse(toml).unwrap_or_else(|error| panic!("{name}: {error}"));
        }
    }

    #[test]
    fn names_are_unique_and_sorted() {
        let names = names().collect::<Vec<_>>();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names, sorted);
    }

    #[test]
    fn a_known_scheme_maps_alacritty_sections_onto_the_palette() {
        let gruvbox = palette("Gruvbox Dark").unwrap();
        assert_eq!(gruvbox.background, rgb(0x282828).into());
        assert_eq!(gruvbox.foreground, rgb(0xebdbb2).into());
        assert_eq!(gruvbox.cursor, rgb(0xebdbb2).into());
        assert_eq!(gruvbox.cursor_text, rgb(0x282828).into());
        assert_eq!(gruvbox.selection, rgb(0x665c54).into());
        assert_eq!(gruvbox.selection_text, Some(rgb(0xebdbb2).into()));
        assert_eq!(gruvbox.normal[1], rgb(0xcc241d).into());
        assert_eq!(gruvbox.bright[4], rgb(0x83a598).into());
        assert!(palette("No Such Scheme").is_none());
    }

    #[test]
    fn colors_accept_both_alacritty_spellings_and_reject_the_rest() {
        assert!(Color::try_from("#ffffff".to_string()).is_ok());
        assert!(Color::try_from("0xffffff".to_string()).is_ok());
        assert!(Color::try_from("ffffff".to_string()).is_err());
        assert!(Color::try_from("#fff".to_string()).is_err());
    }
}
