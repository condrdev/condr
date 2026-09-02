//! Minimum-contrast adjustment for terminal text based on APCA, the
//! Accessible Perceptual Contrast Algorithm. The math follows the published
//! apca-w3 0.0.98G-4g constants: <https://github.com/Myndex/apca-w3>.
//!
//! APCA is polarity-aware and stays accurate on dark backgrounds, which is
//! where WCAG 2.x ratios misjudge terminal themes. Lc 45 is the accepted
//! floor for large fluent text and matches what other terminals default to.

use gpui::Hsla;

// sRGB linearization exponent and luminance coefficients.
const TRC: f32 = 2.4;
const COEFF_RED: f32 = 0.2126729;
const COEFF_GREEN: f32 = 0.7151522;
const COEFF_BLUE: f32 = 0.0721750;

// G-4g exponents per polarity (background/text, normal/reverse).
const NORM_BG: f32 = 0.56;
const NORM_TEXT: f32 = 0.57;
const REV_TEXT: f32 = 0.62;
const REV_BG: f32 = 0.65;

// G-4g black-level clamp, scale and low-contrast rolloff.
const BLACK_THRESHOLD: f32 = 0.022;
const BLACK_CLAMP: f32 = 1.414;
const SCALE: f32 = 1.14;
const LOW_OFFSET: f32 = 0.027;
const MIN_DELTA_Y: f32 = 0.0005;
const LOW_CLIP: f32 = 0.1;

/// Estimated screen luminance of a color, with the APCA black-level clamp.
fn screen_luminance(color: Hsla) -> f32 {
    let rgba = color.to_rgb();
    let y = COEFF_RED * rgba.r.powf(TRC)
        + COEFF_GREEN * rgba.g.powf(TRC)
        + COEFF_BLUE * rgba.b.powf(TRC);
    if y > BLACK_THRESHOLD {
        y
    } else {
        y + (BLACK_THRESHOLD - y).powf(BLACK_CLAMP)
    }
}

/// APCA lightness contrast Lc, roughly -108..=106. Positive means dark text
/// on a light background, negative means light text on a dark background.
pub(crate) fn contrast(text: Hsla, background: Hsla) -> f32 {
    let text_y = screen_luminance(text);
    let background_y = screen_luminance(background);
    if (background_y - text_y).abs() < MIN_DELTA_Y {
        return 0.0;
    }

    let contrast = if background_y > text_y {
        let raw = (background_y.powf(NORM_BG) - text_y.powf(NORM_TEXT)) * SCALE;
        if raw < LOW_CLIP {
            0.0
        } else {
            raw - LOW_OFFSET
        }
    } else {
        let raw = (background_y.powf(REV_BG) - text_y.powf(REV_TEXT)) * SCALE;
        if raw > -LOW_CLIP {
            0.0
        } else {
            raw + LOW_OFFSET
        }
    };
    contrast * 100.0
}

/// Returns a foreground color whose APCA contrast against `background`
/// reaches `minimum` (an absolute Lc value). Keeps the original color when it
/// already passes; otherwise it first moves only the lightness, then trades
/// away saturation, and as a last resort falls back to black or white.
pub(crate) fn ensure_minimum_contrast(foreground: Hsla, background: Hsla, minimum: f32) -> Hsla {
    if minimum <= 0.0 || contrast(foreground, background).abs() >= minimum {
        return foreground;
    }

    for saturation in [
        foreground.s,
        foreground.s * 0.8,
        foreground.s * 0.6,
        foreground.s * 0.4,
        foreground.s * 0.2,
        0.0,
    ] {
        let candidate = with_adjusted_lightness(
            Hsla {
                s: saturation,
                ..foreground
            },
            background,
            minimum,
        );
        if contrast(candidate, background).abs() >= minimum {
            return candidate;
        }
    }

    let black = Hsla {
        h: 0.0,
        s: 0.0,
        l: 0.0,
        a: foreground.a,
    };
    let white = Hsla { l: 1.0, ..black };
    if contrast(white, background).abs() > contrast(black, background).abs() {
        white
    } else {
        black
    }
}

/// Binary-searches the lightness that clears `minimum` while keeping the hue
/// and saturation, moving away from the background's luminance.
fn with_adjusted_lightness(foreground: Hsla, background: Hsla, minimum: f32) -> Hsla {
    let darken = screen_luminance(background) > 0.5;
    let (mut low, mut high) = if darken {
        (0.0, foreground.l)
    } else {
        (foreground.l, 1.0)
    };
    let mut lightness = foreground.l;
    for _ in 0..20 {
        let middle = (low + high) / 2.0;
        let candidate = Hsla {
            l: middle,
            ..foreground
        };
        let contrast = contrast(candidate, background).abs();
        if (contrast - minimum).abs() < 1.0 {
            lightness = middle;
            break;
        }
        if contrast >= minimum {
            // Passing: remember it and move back toward the original color.
            lightness = middle;
            if darken {
                low = middle;
            } else {
                high = middle;
            }
        } else if darken {
            high = middle;
        } else {
            low = middle;
        }
    }
    Hsla {
        l: lightness,
        ..foreground
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::hsla;

    const BLACK: Hsla = hsla(0.0, 0.0, 0.0, 1.0);
    const WHITE: Hsla = hsla(0.0, 0.0, 1.0, 1.0);

    #[test]
    fn contrast_is_polarity_aware_and_zero_for_equal_colors() {
        assert!(contrast(BLACK, WHITE) > 100.0, "dark on light is positive");
        assert!(contrast(WHITE, BLACK) < -100.0, "light on dark is negative");
        assert_eq!(contrast(WHITE, WHITE), 0.0);
        assert_eq!(contrast(BLACK, BLACK), 0.0);
    }

    #[test]
    fn passing_colors_are_returned_unchanged() {
        assert_eq!(ensure_minimum_contrast(WHITE, BLACK, 45.0), WHITE);
        let disabled = hsla(0.6, 0.5, 0.35, 1.0);
        assert_eq!(ensure_minimum_contrast(disabled, BLACK, 0.0), disabled);
    }

    #[test]
    fn low_contrast_text_is_pushed_away_from_the_background() {
        let dark_gray = hsla(0.0, 0.0, 0.12, 1.0);
        let near_black = hsla(0.0, 0.0, 0.05, 1.0);
        let adjusted = ensure_minimum_contrast(near_black, dark_gray, 45.0);
        assert!(
            contrast(adjusted, dark_gray).abs() >= 45.0,
            "adjusted contrast is {}",
            contrast(adjusted, dark_gray)
        );
        assert!(adjusted.l > near_black.l, "dark background lightens text");

        let light_gray = hsla(0.0, 0.0, 0.88, 1.0);
        let near_white = hsla(0.0, 0.0, 0.95, 1.0);
        let adjusted = ensure_minimum_contrast(near_white, light_gray, 45.0);
        assert!(contrast(adjusted, light_gray).abs() >= 45.0);
        assert!(adjusted.l < near_white.l, "light background darkens text");
    }

    #[test]
    fn adjustment_prefers_keeping_the_hue() {
        let dim_red = hsla(0.0, 0.9, 0.18, 1.0);
        let adjusted = ensure_minimum_contrast(dim_red, BLACK, 45.0);
        assert!(contrast(adjusted, BLACK).abs() >= 45.0);
        assert_eq!(adjusted.h, dim_red.h, "hue survives the adjustment");
        assert!(adjusted.s > 0.0, "saturation is only traded when needed");
    }
}
