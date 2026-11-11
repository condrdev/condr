//! The interface's colours, derived from a terminal colour scheme so the sidebars, bars,
//! menus and dialogs sit in the palette of the terminals they frame (ADR 0062). The
//! proportions follow herdr-app's `resolve()`; Kit derives every token not set here from
//! the ones that are.

use gpui_kit::component::theme::ThemeConfig;
use serde_json::json;

use crate::terminal_element::{TerminalPalette, rgb_value};

/// Interface text: a neutral near-white or near-black, as some schemes tint their own
/// foreground.
const DARK_TEXT: u32 = 0xeeeeef;
const LIGHT_TEXT: u32 = 0x1b1b1e;
/// What a light interface's bars lean towards, so they read as paper rather than grey.
const WARM: u32 = 0xe8e8e2;
/// WCAG 2's ratio for body text, which secondary text keeps too.
const MINIMUM_CONTRAST: f32 = 4.5;

/// Whether a scheme is a light one, by its background.
pub(crate) fn is_light(palette: &TerminalPalette) -> bool {
    let background = rgb_value(palette.background);
    let channel = |shift: u32| ((background >> shift) & 0xff) as f32 / 255.;
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0) > 0.5
}

/// The Kit theme for `palette`, light or dark as the scheme is. The terminals' background
/// is the interface's too; the bars are a shade darker (a light scheme's warmer), menus
/// a shade lighter, and lines and hovers a faint wash of the text colour.
pub(crate) fn theme_config(name: &str, palette: &TerminalPalette) -> ThemeConfig {
    let terminal = palette.colors();
    let background = terminal.background;
    let light = is_light(palette);
    let text = if light { LIGHT_TEXT } else { DARK_TEXT };
    let (chrome, popover, raised) = if light {
        (
            mix(background, WARM, 0.6),
            background,
            mix(background, 0, 0.035),
        )
    } else {
        (
            mix(background, 0, 0.32),
            mix(background, 0xffffff, 0.06),
            mix(background, 0xffffff, 0.09),
        )
    };
    let (hover, selected, border, input) = if light {
        ("#0000000c", "#00000012", "#00000017", "#00000026")
    } else {
        ("#ffffff0e", "#ffffff17", "#ffffff18", "#ffffff24")
    };
    let surfaces = [background, chrome, popover];
    let foreground = legible(text, &surfaces, light);
    let muted = legible(
        mix(background, text, if light { 0.66 } else { 0.6 }),
        &surfaces,
        light,
    );
    let accent = terminal.ansi[4];
    // Whichever reads better on the accent: a light scheme's blue can be pale.
    let on_accent = if contrast(background, accent) >= contrast(foreground, accent) {
        background
    } else {
        foreground
    };
    let ansi = |index: usize| hex(terminal.ansi[index]);
    let config = json!({
        "name": name,
        "mode": if light { "light" } else { "dark" },
        "colors": {
            "background": hex(background),
            "foreground": hex(foreground),
            "border": border,
            "input.border": input,
            "ring": hex(mix(background, text, 0.7)),
            "selection.background": hex(rgb_value(palette.selection)),
            "muted.background": hex(raised),
            "muted.foreground": hex(muted),
            "popover.background": hex(popover),
            "popover.foreground": hex(foreground),
            "primary.background": hex(accent),
            "primary.foreground": hex(on_accent),
            "secondary.background": hex(raised),
            "secondary.foreground": hex(foreground),
            "accent.background": selected,
            "accent.foreground": hex(foreground),
            "list.hover.background": hover,
            "list.active.background": selected,
            "sidebar.background": hex(chrome),
            "sidebar.foreground": hex(foreground),
            "sidebar.border": border,
            "sidebar.accent.background": selected,
            "sidebar.accent.foreground": hex(foreground),
            "title_bar.background": hex(chrome),
            "title_bar.border": border,
            "tab_bar.background": hex(chrome),
            "status_bar.background": hex(chrome),
            "status_bar.border": border,
            "window.border": border,
            "base.red": ansi(1),
            "base.green": ansi(2),
            "base.yellow": ansi(3),
            "base.blue": ansi(4),
            "base.magenta": ansi(5),
            "base.cyan": ansi(6),
        },
    });
    serde_json::from_value(config).expect("a derived theme is a valid Kit theme")
}

/// `color`, pushed towards white (black on a light scheme) until it keeps
/// [`MINIMUM_CONTRAST`] on every one of `surfaces`.
fn legible(color: u32, surfaces: &[u32], light: bool) -> u32 {
    let extreme = if light { 0 } else { 0xffffff };
    (0..=20)
        .map(|step| mix(color, extreme, step as f32 / 20.))
        .find(|&candidate| {
            surfaces
                .iter()
                .all(|&surface| contrast(candidate, surface) >= MINIMUM_CONTRAST)
        })
        .unwrap_or(extreme)
}

/// `a` moved `t` of the way to `b`, channel by channel.
fn mix(a: u32, b: u32, t: f32) -> u32 {
    [16, 8, 0].iter().fold(0, |value, shift| {
        let (from, to) = (((a >> shift) & 0xff) as f32, ((b >> shift) & 0xff) as f32);
        value | (((from * (1. - t) + to * t).round() as u32) << shift)
    })
}

/// WCAG 2's contrast ratio, 1 to 21.
fn contrast(a: u32, b: u32) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn luminance(color: u32) -> f32 {
    let channel = |shift: u32| {
        let value = ((color >> shift) & 0xff) as f32 / 255.;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

fn hex(color: u32) -> String {
    format!("#{color:06x}")
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::theme::{Theme, ThemeColor};
    use std::rc::Rc;

    use super::*;

    fn applied(palette: &TerminalPalette) -> ThemeColor {
        let mut theme = Theme::default();
        theme.apply_config(&Rc::new(theme_config("Test", palette)));
        theme.colors
    }

    #[test]
    fn the_interface_shares_the_terminal_background_and_frames_it_darker() {
        let palette = crate::color_scheme::palette("Gruvbox Dark").unwrap();
        let colors = applied(&palette);
        let same = |interface, terminal| rgb_value(interface) == rgb_value(terminal);
        assert!(same(colors.background, palette.background));
        assert_eq!(rgb_value(colors.sidebar), mix(0x282828, 0, 0.32));
        assert!(same(colors.title_bar, colors.sidebar));
        assert!(
            same(colors.primary, palette.normal[4]),
            "the accent is the scheme's blue"
        );
        assert!(
            same(colors.danger, palette.normal[1]),
            "states take the scheme's red"
        );
        assert!(same(colors.success, palette.normal[2]));
        assert!(same(colors.warning, palette.normal[3]));
    }

    #[test]
    fn a_light_scheme_gives_a_light_interface_with_warm_bars() {
        let palette = crate::color_scheme::palette("GitHub Light Default").unwrap();
        assert!(is_light(&palette));
        let config = theme_config("Test", &palette);
        assert!(!config.mode.is_dark());
        let colors = applied(&palette);
        assert_eq!(rgb_value(colors.sidebar), mix(0xffffff, WARM, 0.6));
    }

    #[test]
    fn text_stays_legible_on_every_surface_of_every_built_in_scheme() {
        for name in crate::color_scheme::names() {
            let palette = crate::color_scheme::palette(name).unwrap();
            let colors = applied(&palette);
            for surface in [colors.background, colors.sidebar, colors.popover] {
                for (role, text) in [
                    ("foreground", colors.foreground),
                    ("muted", colors.muted_foreground),
                ] {
                    let ratio = contrast(rgb_value(text), rgb_value(surface));
                    assert!(
                        ratio >= MINIMUM_CONTRAST,
                        "{name}: {role} text has {ratio:.2}:1"
                    );
                }
            }
        }
    }

    #[test]
    fn legible_leaves_readable_text_alone_and_pushes_faint_text_out() {
        assert_eq!(legible(0xeeeeef, &[0x111111], false), 0xeeeeef);
        let faint = legible(0x333333, &[0x111111], false);
        assert!(contrast(faint, 0x111111) >= MINIMUM_CONTRAST);
        // Out of reach even in white: white is the best there is.
        assert_eq!(legible(0x999999, &[0x777777], false), 0xffffff);
    }
}
