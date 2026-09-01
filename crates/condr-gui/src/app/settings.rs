use super::*;

/// The dialog is sized in pixels because `Dialog::w` takes pixels and does not
/// clamp itself to the viewport, so a small window would otherwise clip it.
const SETTINGS_DIALOG_WIDTH: Pixels = px(860.);
const SETTINGS_DIALOG_HEIGHT: Pixels = px(520.);
const SETTINGS_DIALOG_MARGIN: Pixels = px(48.);

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

/// The single place that hands an Appearance to the theme.
pub(super) fn apply_appearance(appearance: Appearance, window: Option<&mut Window>, cx: &mut App) {
    match appearance {
        Appearance::System => Theme::sync_system_appearance(window, cx),
        Appearance::Light => Theme::change(ThemeMode::Light, window, cx),
        Appearance::Dark => Theme::change(ThemeMode::Dark, window, cx),
    }
}

impl Condr {
    pub(super) fn set_appearance(&mut self, appearance: Appearance, cx: &mut Context<Self>) {
        if self.appearance == appearance {
            return;
        }
        self.appearance = appearance;
        // Condr renders the dialog layer itself, so notifying it repaints the whole window.
        apply_appearance(appearance, None, cx);
        self.save_appearance();
        cx.notify();
    }

    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let owner = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            let owner = owner.clone();
            window.open_dialog(cx, move |dialog, window, _| {
                let owner = owner.clone();
                let viewport = window.viewport_size();
                let width = SETTINGS_DIALOG_WIDTH.min(viewport.width - SETTINGS_DIALOG_MARGIN);
                let height = SETTINGS_DIALOG_HEIGHT.min(viewport.height - SETTINGS_DIALOG_MARGIN);
                dialog
                    .title("Settings")
                    .w(width.max(px(1.)))
                    .content(move |content, _, _| {
                        content
                            .h(height.max(px(1.)))
                            .child(Settings::new("condr-settings").page(appearance_page(&owner)))
                    })
            });
        });
    }
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
                        move |cx| {
                            selected_owner
                                .upgrade()
                                .map(|owner| owner.read(cx).appearance)
                                .unwrap_or_default()
                                .as_str()
                                .into()
                        },
                        move |value: SharedString, cx| {
                            let appearance = Appearance::from_str(&value);
                            let _ = select_owner
                                .update(cx, |this, cx| this.set_appearance(appearance, cx));
                        },
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
