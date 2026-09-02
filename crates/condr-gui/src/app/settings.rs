use super::*;

// Sized in rems so the window zooms with the base font, resolved against the main
// window's rem size when it opens.
const SETTINGS_WINDOW_WIDTH: Rems = rems(54.);
const SETTINGS_WINDOW_HEIGHT: Rems = rems(34.);
// Wide enough for the page sidebar plus the Colors select.
const SETTINGS_WINDOW_MIN_WIDTH: Rems = rems(40.);
const SETTINGS_WINDOW_MIN_HEIGHT: Rems = rems(20.);

/// Centered over the main window and kept on that window's display, so a main
/// window on a secondary screen gets its Settings there too.
fn settings_window_bounds(window: &Window, size: Size<Pixels>, cx: &App) -> Bounds<Pixels> {
    let main = window.bounds();
    let mut origin = point(
        main.origin.x + (main.size.width - size.width) / 2.,
        main.origin.y + (main.size.height - size.height) / 2.,
    );
    if let Some(display) = window.display(cx) {
        let screen = display.bounds();
        origin.x = origin.x.min(screen.right() - size.width).max(screen.left());
        origin.y = origin
            .y
            .min(screen.bottom() - size.height)
            .max(screen.top());
    }
    Bounds::new(origin, size)
}

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

/// The font every Terminal renders with. Lives on the Appearance page.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct TerminalFont {
    pub family: SharedString,
    pub size: f32,
}

impl TerminalFont {
    pub(super) const MIN_SIZE: f32 = 6.;
    pub(super) const MAX_SIZE: f32 = 72.;

    pub(super) fn clamp_size(size: f32) -> f32 {
        if size.is_finite() {
            size.clamp(Self::MIN_SIZE, Self::MAX_SIZE)
        } else {
            Self::default().size
        }
    }

    /// What the theme and the config file get: an empty family falls back to the
    /// default and the size is clamped. The typed value itself stays as is.
    pub(super) fn normalized(&self) -> Self {
        Self {
            family: if self.family.trim().is_empty() {
                Self::default().family
            } else {
                self.family.clone()
            },
            size: Self::clamp_size(self.size),
        }
    }
}

impl Default for TerminalFont {
    fn default() -> Self {
        let theme = Theme::default();
        Self {
            family: theme.mono_font_family.clone(),
            size: theme.mono_font_size.as_f32(),
        }
    }
}

/// The Terminal reads its font from the theme's mono font, which a mode change keeps.
pub(super) fn apply_terminal_font(font: &TerminalFont, cx: &mut App) {
    let theme = Theme::global_mut(cx);
    theme.mono_font_family = font.family.clone();
    theme.mono_font_size = px(font.size);
    // The Base layer keeps its own projection of the typography tokens.
    Theme::sync_base(cx);
    cx.refresh_windows();
}

/// Terminal color scheme names come from the built-in collection; an empty or
/// unknown name selects the built-in default palette.
pub(super) fn apply_terminal_color_scheme(name: &str, cx: &mut App) {
    cx.set_global(crate::color_scheme::palette(name).unwrap_or_default());
    cx.refresh_windows();
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
            // The Settings window shows the new mode too.
            cx.refresh_windows();
        });
    }

    /// Keeps the value as typed so the fields never rewrite a half-edited entry;
    /// the theme and the config file receive the normalized form.
    pub(super) fn set_terminal_font(&mut self, font: TerminalFont, cx: &mut Context<Self>) {
        if self.terminal_font == font {
            return;
        }
        self.terminal_font = font;
        self.save_terminal_font();
        apply_terminal_font(&self.terminal_font.normalized(), cx);
    }

    pub(super) fn set_terminal_color_scheme(&mut self, name: SharedString, cx: &mut Context<Self>) {
        if self.terminal_color_scheme == name {
            return;
        }
        self.terminal_color_scheme = name;
        self.save_terminal_color_scheme();
        apply_terminal_color_scheme(&self.terminal_color_scheme, cx);
    }

    /// Settings opens in its own window, as Zed does, so it can be moved aside while
    /// the real Panes behind it show every change live. A second open re-activates it.
    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(existing) = self.settings_window
            && existing
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let rem = window.rem_size();
        let window_size = size(
            SETTINGS_WINDOW_WIDTH.to_pixels(rem),
            SETTINGS_WINDOW_HEIGHT.to_pixels(rem),
        );
        let min_size = size(
            SETTINGS_WINDOW_MIN_WIDTH.to_pixels(rem),
            SETTINGS_WINDOW_MIN_HEIGHT.to_pixels(rem),
        );
        let bounds = settings_window_bounds(window, window_size, cx);
        let main_window = self.window_handle;
        let owner = cx.weak_entity();
        // Opening a window needs the App without this entity on the stack.
        cx.defer(move |cx| {
            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("Condr — Settings".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(min_size),
                ..Default::default()
            };
            let view_owner = owner.clone();
            let opened = cx.open_window(options, |window, cx| {
                let view = cx.new(|cx| SettingsWindow::new(view_owner, window, cx));
                view.read(cx).focus_handle.clone().focus(window, cx);
                cx.new(|cx| Root::new(view, window, cx))
            });
            let _ = owner.update(cx, |this, cx| match opened {
                Ok(handle) => {
                    this.settings_window = Some(handle);
                    // Closing the main window closes Settings too; otherwise it would
                    // keep the process alive with nothing left to configure.
                    this._settings_window_closed = Some(cx.on_window_closed(move |cx, _| {
                        if !cx.windows().contains(&main_window) {
                            let _ = handle.update(cx, |_, window, _| window.remove_window());
                        }
                    }));
                }
                Err(error) => this.app_error = Some(format!("Failed to open Settings: {error}")),
            });
        });
    }
}

/// The Colors entry standing for the built-in palette, stored as an empty name.
const DEFAULT_COLOR_SCHEME_LABEL: &str = "Default";

fn color_scheme_name(label: &SharedString) -> SharedString {
    if label == DEFAULT_COLOR_SCHEME_LABEL {
        SharedString::default()
    } else {
        label.clone()
    }
}

type ColorSchemeSelect = SelectState<SearchableVec<SharedString>>;

/// The Settings window's root. Every value lives on `Condr`; it only owns the widget
/// state a searchable list of 600 schemes needs to scroll to and filter its selection.
pub(super) struct SettingsWindow {
    owner: WeakEntity<Condr>,
    focus_handle: FocusHandle,
    color_scheme: Entity<ColorSchemeSelect>,
    /// The Licenses page text, in a read-only editor because it is far too long
    /// for a plain text element.
    licenses: Entity<EditorState>,
}

/// Generated by `script/generate-licenses`; rerun it after adding assets or crates.
const LICENSES: &str = include_str!("../../assets/licenses.md");

impl SettingsWindow {
    fn new(owner: WeakEntity<Condr>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let labels = std::iter::once(DEFAULT_COLOR_SCHEME_LABEL.into())
            .chain(crate::color_scheme::names().map(SharedString::from))
            .collect::<Vec<SharedString>>();
        let current = owner
            .upgrade()
            .map(|owner| owner.read(cx).terminal_color_scheme.clone())
            .unwrap_or_default();
        let selected = labels
            .iter()
            .position(|label| color_scheme_name(label) == current)
            .map(IndexPath::new);
        let color_scheme = cx.new(|cx| {
            SelectState::new(SearchableVec::new(labels), selected, window, cx).searchable(true)
        });
        cx.subscribe(
            &color_scheme,
            |this, _, event: &SelectEvent<SearchableVec<SharedString>>, cx| {
                let SelectEvent::Confirm(Some(label)) = event else {
                    return;
                };
                let name = color_scheme_name(label);
                let _ = this
                    .owner
                    .update(cx, |owner, cx| owner.set_terminal_color_scheme(name, cx));
            },
        )
        .detach();
        let licenses = cx.new(|cx| {
            EditorState::new(window, cx)
                .default_value(LICENSES)
                .soft_wrap(true)
                .line_number(false)
                // License texts are indented prose, not code: no guides, no fold gutter.
                .indent_guides(false)
                .folding(false)
                .searchable(true)
        });
        Self {
            owner,
            focus_handle: cx.focus_handle(),
            color_scheme,
            licenses,
        }
    }
}

fn licenses_page(licenses: &Entity<EditorState>) -> SettingPage {
    let licenses = licenses.clone();
    SettingPage::new("Licenses")
        .icon(IconName::BookOpen)
        .group(
            SettingGroup::new().item(SettingItem::render(move |_, _, _| {
                // Plain text: no border, background or line numbers. It stays an
                // Editor only because a 900 KB text needs virtualized rendering.
                Editor::new(&licenses)
                    .readonly(true)
                    .appearance(false)
                    .bordered(false)
                    .h(rems(26.))
            })),
        )
}

impl Render for SettingsWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("condr-settings-window")
            .debug_selector(|| "settings-content".into())
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_key_down(|event, window, _| {
                if event.keystroke.key == "escape" {
                    window.remove_window();
                }
            })
            .child(
                Settings::new("condr-settings")
                    .page(appearance_page(&self.owner, &self.color_scheme))
                    .page(licenses_page(&self.licenses)),
            )
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

fn terminal_font(owner: &WeakEntity<Condr>, cx: &App) -> TerminalFont {
    owner
        .upgrade()
        .map(|owner| owner.read(cx).terminal_font.clone())
        .unwrap_or_default()
}

fn appearance_page(
    owner: &WeakEntity<Condr>,
    color_scheme: &Entity<ColorSchemeSelect>,
) -> SettingPage {
    let color_scheme = color_scheme.clone();
    let reset_select = color_scheme.clone();
    let dirty_owner = owner.clone();
    let reset_owner = owner.clone();
    let selected_owner = owner.clone();
    let select_owner = owner.clone();
    let options = Appearance::ALL
        .map(|appearance| (appearance.as_str().into(), appearance.label().into()))
        .to_vec();
    let default_font = TerminalFont::default();
    let family_get = owner.clone();
    let family_set = owner.clone();
    let size_get = owner.clone();
    let size_set = owner.clone();
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
                    )
                    .default_value(Appearance::default().as_str()),
                )
                .description("Follow the system appearance, or pick one."),
            ),
        )
        .group(
            SettingGroup::new()
                .title("Terminal")
                .item(
                    SettingItem::new(
                        "Font",
                        SettingField::input(
                            move |cx| terminal_font(&family_get, cx).family,
                            move |value: SharedString, cx| {
                                let _ = family_set.update(cx, |this, cx| {
                                    let font = TerminalFont {
                                        family: value,
                                        ..this.terminal_font.clone()
                                    };
                                    this.set_terminal_font(font, cx);
                                });
                            },
                        )
                        .default_value(default_font.family.clone()),
                    )
                    .description("Font family used by every terminal."),
                )
                .item(
                    SettingItem::new(
                        "Font size",
                        SettingField::number_input(
                            NumberFieldOptions {
                                min: f64::from(TerminalFont::MIN_SIZE),
                                max: f64::from(TerminalFont::MAX_SIZE),
                                step: 1.,
                            },
                            move |cx| f64::from(terminal_font(&size_get, cx).size),
                            move |value: f64, cx| {
                                let _ = size_set.update(cx, |this, cx| {
                                    let font = TerminalFont {
                                        size: value as f32,
                                        ..this.terminal_font.clone()
                                    };
                                    this.set_terminal_font(font, cx);
                                });
                            },
                        )
                        .default_value(f64::from(default_font.size)),
                    )
                    .description("In pixels."),
                )
                .item(
                    SettingItem::new(
                        "Colors",
                        // A searchable Select: the list needs filtering, and it opens
                        // scrolled to the current choice.
                        SettingField::render(move |_, _, _| {
                            // The field slot shrinks to content, so the trigger and the
                            // menu need a width that fits the longest scheme name. Rems,
                            // like the window itself; the window's minimum width leaves
                            // room for this beside the page sidebar.
                            Select::new(&color_scheme)
                                .w(rems(17.5))
                                .menu_width(rems(20.))
                                .menu_max_h(rems(22.5))
                        })
                        .on_reset(
                            move |cx| {
                                dirty_owner.upgrade().is_some_and(|owner| {
                                    !owner.read(cx).terminal_color_scheme.is_empty()
                                })
                            },
                            move |window, cx| {
                                let _ = reset_owner.update(cx, |this, cx| {
                                    this.set_terminal_color_scheme(SharedString::default(), cx)
                                });
                                reset_select.update(cx, |select, cx| {
                                    select.set_selected_value(
                                        &DEFAULT_COLOR_SCHEME_LABEL.into(),
                                        window,
                                        cx,
                                    );
                                });
                            },
                        ),
                    )
                    .description("Terminal color schemes."),
                ),
        )
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: it would glob in `gpui::test` and shadow the built-in attribute.
    #[cfg(feature = "test-support")]
    use gpui_component::ActiveTheme as _;

    use super::{Appearance, TerminalFont};
    #[cfg(feature = "test-support")]
    use super::{apply_appearance, apply_terminal_font};

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

    #[test]
    fn terminal_font_size_stays_within_bounds() {
        assert_eq!(TerminalFont::clamp_size(1.), TerminalFont::MIN_SIZE);
        assert_eq!(TerminalFont::clamp_size(500.), TerminalFont::MAX_SIZE);
        assert_eq!(TerminalFont::clamp_size(14.), 14.);
        assert_eq!(
            TerminalFont::clamp_size(f32::NAN),
            TerminalFont::default().size
        );

        // Typed values stay as typed; only the normalized copy is corrected.
        let typed = TerminalFont {
            family: "  ".into(),
            size: 1.,
        };
        let normalized = typed.normalized();
        assert_eq!(normalized.family, TerminalFont::default().family);
        assert_eq!(normalized.size, TerminalFont::MIN_SIZE);
        assert_eq!(typed.family.as_ref(), "  ");
    }

    #[cfg(feature = "test-support")]
    #[gpui::test]
    fn the_terminal_font_survives_an_appearance_change(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            let font = TerminalFont {
                family: "Cascadia Mono".into(),
                size: 17.,
            };
            apply_terminal_font(&font, cx);
            apply_appearance(Appearance::Dark, None, cx);
            apply_appearance(Appearance::Light, None, cx);
            assert_eq!(cx.theme().mono_font_family.as_ref(), "Cascadia Mono");
            assert_eq!(cx.theme().mono_font_size, gpui::px(17.));
        });
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
