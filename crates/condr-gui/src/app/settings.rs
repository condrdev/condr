use super::*;

// Sized in rems so the dialog zooms with the base font, then resolved to the pixels
// Dialog takes. These bound the whole dialog, not its content: Dialog does not clamp
// itself to the viewport, but it does give its content whatever it has left, so the
// content never has to know what the title and padding cost.
const SETTINGS_DIALOG_WIDTH: Rems = rems(54.);
const SETTINGS_DIALOG_HEIGHT: Rems = rems(34.);
const SETTINGS_DIALOG_MARGIN: Rems = rems(3.);

/// The GUI appearance preference. `System` follows the OS; the other two pin a mode.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub(super) const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    /// Unrecognized values follow the system rather than failing to load.
    pub(super) fn from_str(value: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|appearance| appearance.as_str() == value)
            .unwrap_or_default()
    }
}

/// Applies a chosen Appearance: the native window-chrome override plus the theme.
///
/// Forcing an appearance stops the platform from tracking system light/dark changes,
/// so `System` clears the override instead of setting one. A system appearance change
/// goes through [`sync_theme_with_system`], which leaves the override alone.
pub(super) fn apply_appearance(appearance: Appearance, window: Option<&mut Window>, cx: &mut App) {
    match appearance {
        Appearance::System => {
            cx.set_window_appearance(None);
            Theme::sync_system_appearance(window, cx);
        }
        Appearance::Light => {
            cx.set_window_appearance(Some(WindowAppearance::Light));
            Theme::change(ThemeMode::Light, window, cx);
        }
        Appearance::Dark => {
            cx.set_window_appearance(Some(WindowAppearance::Dark));
            Theme::change(ThemeMode::Dark, window, cx);
        }
    }
}

/// Re-resolves `System` after the operating system changed its appearance.
pub(super) fn sync_theme_with_system(window: &mut Window, cx: &mut App) {
    Theme::sync_system_appearance(Some(window), cx);
}

impl Condr {
    pub(super) fn set_appearance(&mut self, appearance: Appearance, cx: &mut Context<Self>) {
        if self.appearance == appearance {
            return;
        }
        self.appearance = appearance;
        self.save_appearance();
        let handle = self.window_handle;
        // Resolving System reads the Window's own appearance, which Linux reports more
        // reliably than the app-global one. GPUI takes the Window out of its table for
        // the duration of an update, so it is unreachable from here — this runs inside
        // one — and the apply has to wait for the current update to finish. A closed
        // window has nothing left to theme. Applying refreshes the Window, which
        // repaints the dialog along with everything else, so there is nothing to notify.
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                apply_appearance(appearance, Some(window), cx)
            });
        });
    }

    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let owner = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let owner = owner.clone();
            window.open_dialog(cx, move |dialog, window, _| {
                let owner = owner.clone();
                let rem = window.rem_size();
                let viewport = window.viewport_size();
                let margin = SETTINGS_DIALOG_MARGIN.to_pixels(rem);
                let width = SETTINGS_DIALOG_WIDTH
                    .to_pixels(rem)
                    .min(viewport.width - margin - margin)
                    .max(px(1.));
                let height = SETTINGS_DIALOG_HEIGHT
                    .to_pixels(rem)
                    .min(viewport.height - margin - margin)
                    .max(px(1.));
                dialog
                    .title("Settings")
                    .w(width)
                    .h(height)
                    .margin_top(margin)
                    .content(move |content, _, _| {
                        content.child(
                            div()
                                .id("condr-settings-content")
                                .debug_selector(|| "settings-content".into())
                                .size_full()
                                .child(
                                    Settings::new("condr-settings").page(appearance_page(&owner)),
                                ),
                        )
                    })
            });
        });
    }
}

/// What the Mode dropdown shows. Its getter only ever sees an `&App`.
pub(super) fn selected_appearance(owner: &WeakEntity<Condr>, cx: &App) -> SharedString {
    owner
        .upgrade()
        .map(|owner| owner.read(cx).appearance)
        .unwrap_or_default()
        .as_str()
        .into()
}

/// What the Mode dropdown does. Its setter only ever sees an `&mut App`.
pub(super) fn select_appearance(owner: &WeakEntity<Condr>, value: &str, cx: &mut App) {
    let appearance = Appearance::from_str(value);
    let _ = owner.update(cx, |this, cx| this.set_appearance(appearance, cx));
}

fn appearance_page(owner: &WeakEntity<Condr>) -> SettingPage {
    let selected_owner = owner.clone();
    let select_owner = owner.clone();
    let options = Appearance::ALL
        .map(|appearance| (appearance.as_str().into(), appearance.label().into()))
        .to_vec();
    SettingPage::new("Appearance")
        .icon(IconName::Palette)
        .group(
            SettingGroup::new().item(
                SettingItem::new(
                    // The page is already named Appearance, and CONTEXT.md avoids "Theme".
                    "Mode",
                    SettingField::dropdown(
                        options,
                        move |cx| selected_appearance(&selected_owner, cx),
                        move |value: SharedString, cx| select_appearance(&select_owner, &value, cx),
                    ),
                )
                .description("Follow the system appearance, or pick one."),
            ),
        )
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: it would glob in `gpui::test` and shadow the built-in attribute.
    #[cfg(feature = "test-support")]
    use gpui_component::ActiveTheme as _;

    use super::Appearance;
    #[cfg(feature = "test-support")]
    use super::apply_appearance;

    #[test]
    fn appearance_round_trips_through_its_stored_name() {
        for appearance in Appearance::ALL {
            assert_eq!(Appearance::from_str(appearance.as_str()), appearance);
        }
    }

    #[test]
    fn unknown_appearance_names_follow_the_system() {
        assert_eq!(Appearance::from_str("solarized"), Appearance::System);
        assert_eq!(Appearance::from_str(""), Appearance::System);
    }

    #[cfg(feature = "test-support")]
    #[gpui::test]
    fn applying_an_appearance_pins_the_theme_mode(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);

            apply_appearance(Appearance::Dark, None, cx);
            assert!(cx.theme().is_dark());

            apply_appearance(Appearance::Light, None, cx);
            assert!(!cx.theme().is_dark());
        });
    }
}
