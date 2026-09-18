use super::*;

/// How long after the last font change the config file is written, so a run of
/// stepper clicks rewrites it once.
pub(super) const FONT_SAVE_DEBOUNCE: Duration = Duration::from_millis(300);

/// The GUI appearance preference. `System` follows the OS; the other two pin a mode.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::app) enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub(in crate::app) const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub(in crate::app) fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub(in crate::app) fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Light => "Light",
            Self::Dark => "Dark",
        }
    }

    /// Unrecognized values follow the system rather than failing to load.
    pub(in crate::app) fn from_str(value: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|appearance| appearance.as_str() == value)
            .unwrap_or_default()
    }
}

/// The font every Terminal renders with. Lives on the Appearance page.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::app) struct TerminalFont {
    pub family: SharedString,
    pub size: f32,
}

impl TerminalFont {
    pub(in crate::app) const MIN_SIZE: f32 = 6.;
    pub(in crate::app) const MAX_SIZE: f32 = 72.;

    /// Whole pixels within bounds: the stepper moves by one and shows integers, so a
    /// hand-edited `13.5` becomes 14 rather than a value the UI cannot display.
    pub(in crate::app) fn clamp_size(size: f32) -> f32 {
        if size.is_finite() {
            size.round().clamp(Self::MIN_SIZE, Self::MAX_SIZE)
        } else {
            Self::default().size
        }
    }

    /// What the theme and the config file get: an empty family falls back to the
    /// default and the size is clamped. The typed value itself stays as is.
    pub(in crate::app) fn normalized(&self) -> Self {
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
pub(in crate::app) fn apply_terminal_font(font: &TerminalFont, cx: &mut App) {
    let theme = Theme::global_mut(cx);
    theme.mono_font_family = font.family.clone();
    theme.mono_font_size = px(font.size);
    // The Base layer keeps its own projection of the typography tokens.
    Theme::sync_base(cx);
    cx.refresh_windows();
}

/// Terminal color scheme names come from the built-in collection; an empty or
/// unknown name selects the built-in default palette.
pub(in crate::app) fn apply_terminal_color_scheme(name: &str, cx: &mut App) {
    cx.set_global(crate::color_scheme::palette(name).unwrap_or_default());
    cx.refresh_windows();
}

/// Applies a chosen Appearance: the native window-chrome override plus the theme.
///
/// Forcing an appearance stops the platform from tracking system light/dark changes,
/// so `System` clears the override instead of setting one. A system appearance change
/// goes through [`sync_theme_with_system`], which leaves the override alone.
pub(in crate::app) fn apply_appearance(
    appearance: Appearance,
    window: Option<&mut Window>,
    cx: &mut App,
) {
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
pub(in crate::app) fn sync_theme_with_system(window: &mut Window, cx: &mut App) {
    Theme::sync_system_appearance(Some(window), cx);
}

/// The Colors entry standing for the built-in palette, stored as an empty name.
pub(super) const DEFAULT_COLOR_SCHEME_LABEL: &str = "Default";

pub(super) fn color_scheme_name(label: &SharedString) -> SharedString {
    if label == DEFAULT_COLOR_SCHEME_LABEL {
        SharedString::default()
    } else {
        label.clone()
    }
}

pub(super) type ColorSchemeSelect = SelectState<SearchableVec<SharedString>>;

/// What the Mode dropdown shows. Its getter only ever sees an `&App`.
// The field callbacks below are named so tests can drive them the way the widgets
// do; the widgets themselves belong to GPUI Kit and expose no test hooks.
pub(in crate::app) fn selected_appearance(owner: &WeakEntity<Condr>, cx: &App) -> SharedString {
    owner
        .upgrade()
        .map(|owner| owner.read(cx).appearance)
        .unwrap_or_default()
        .as_str()
        .into()
}

/// What the Mode dropdown does. Its setter only ever sees an `&mut App`.
pub(in crate::app) fn select_appearance(owner: &WeakEntity<Condr>, value: &str, cx: &mut App) {
    let appearance = Appearance::from_str(value);
    let _ = owner.update(cx, |this, cx| this.set_appearance(appearance, cx));
}

/// What the Font field shows; tests read it without the widget.
#[cfg(all(test, feature = "test-support"))]
pub(in crate::app) fn terminal_font_family(
    settings: &Entity<SettingsWindow>,
    cx: &App,
) -> SharedString {
    settings.read(cx).font_family.draft.clone()
}

/// What typing a family and leaving the Font field does; tests call it without the widget.
#[cfg(all(test, feature = "test-support"))]
pub(in crate::app) fn select_terminal_font_family(
    settings: &Entity<SettingsWindow>,
    family: SharedString,
    cx: &mut App,
) {
    settings.update(cx, |this, cx| {
        this.font_family.draft = family;
        this.commit(TextFieldId::FontFamily, cx);
    });
}

/// What the Font size field shows.
pub(in crate::app) fn terminal_font_size(settings: &Entity<SettingsWindow>, cx: &App) -> f64 {
    f64::from(settings.read(cx).font_draft.size)
}

/// What the Font size field does on every change.
pub(in crate::app) fn select_terminal_font_size(
    settings: &Entity<SettingsWindow>,
    size: f64,
    cx: &mut App,
) {
    settings.update(cx, |this, cx| {
        this.font_draft.size = size as f32;
        this.commit_font(cx);
    });
}

/// What the Font size stepper buttons do: one pixel at a time, within bounds.
pub(in crate::app) fn step_terminal_font_size(
    settings: &Entity<SettingsWindow>,
    delta: f32,
    cx: &mut App,
) {
    settings.update(cx, |this, cx| {
        this.font_draft.size = TerminalFont::clamp_size(this.font_draft.size + delta);
        this.commit_font(cx);
    });
}

/// Whether Reset All has anything to do for Colors.
pub(in crate::app) fn color_scheme_is_dirty(owner: &WeakEntity<Condr>, cx: &App) -> bool {
    owner
        .upgrade()
        .is_some_and(|owner| !owner.read(cx).terminal_color_scheme.is_empty())
}

/// What Reset All does for Colors: back to the built-in palette, in both the
/// preference and the select widget.
pub(in crate::app) fn reset_color_scheme(
    owner: &WeakEntity<Condr>,
    select: &Entity<ColorSchemeSelect>,
    window: &mut Window,
    cx: &mut App,
) {
    let _ = owner.update(cx, |this, cx| {
        this.set_terminal_color_scheme(SharedString::default(), cx)
    });
    select.update(cx, |select, cx| {
        select.set_selected_value(&DEFAULT_COLOR_SCHEME_LABEL.into(), window, cx);
    });
}

/// This Client's appearance and terminal rendering preferences.
pub(super) fn appearance_page(
    owner: &WeakEntity<Condr>,
    settings: &Entity<SettingsWindow>,
    color_scheme: &Entity<ColorSchemeSelect>,
) -> SettingPage {
    let reset_select = color_scheme.clone();
    let dirty_owner = owner.clone();
    let reset_owner = owner.clone();
    let selected_owner = owner.clone();
    let select_owner = owner.clone();
    let options = Appearance::ALL
        .map(|appearance| (appearance.as_str().into(), appearance.label().into()))
        .to_vec();
    let default_font = TerminalFont::default();
    let size_get = settings.clone();
    let size_set = settings.clone();
    let size_dirty = settings.clone();
    let default_size = default_font.size;
    let scheme_select = color_scheme.clone();
    SettingPage::new("Appearance")
        .icon(IconName::Palette)
        .group(
            SettingGroup::new().item(
                SettingItem::new(
                    "Theme",
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
                        text_field_row(
                            settings,
                            TextFieldId::FontFamily,
                            default_font.family.clone(),
                        ),
                    )
                    .description("Font family used by every terminal."),
                )
                .item(
                    SettingItem::new(
                        "Font size",
                        // A stepper rather than a text field: the number widget rewrites
                        // half-typed values that leave its range, and one pixel at a time
                        // is how a font size gets tuned anyway.
                        SettingField::render(move |_, _, cx| {
                            let size = terminal_font_size(&size_get, cx);
                            let step = |id: &'static str, icon, delta: f32, enabled: bool| {
                                let settings = size_get.clone();
                                let label = format!("{id} font size");
                                let label = format!("{}{}", label[..1].to_uppercase(), &label[1..]);
                                div()
                                    .debug_selector(move || format!("terminal-font-size-{id}"))
                                    .child(
                                        Button::new(format!("terminal-font-size-{id}"))
                                            .icon(icon)
                                            .small()
                                            .outline()
                                            .tooltip(label.clone())
                                            .accessibility_label(label)
                                            .disabled(!enabled)
                                            .on_click(move |_, _, cx| {
                                                step_terminal_font_size(&settings, delta, cx)
                                            }),
                                    )
                            };
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(step(
                                    "decrease",
                                    IconName::Minus,
                                    -1.,
                                    size > f64::from(TerminalFont::MIN_SIZE),
                                ))
                                .child(
                                    div()
                                        .min_w(rems(2.))
                                        .text_center()
                                        .child(format!("{size:.0}")),
                                )
                                .child(step(
                                    "increase",
                                    IconName::Plus,
                                    1.,
                                    size < f64::from(TerminalFont::MAX_SIZE),
                                ))
                        })
                        .on_reset(
                            move |cx| {
                                terminal_font_size(&size_dirty, cx) != f64::from(default_size)
                            },
                            move |_, cx| {
                                select_terminal_font_size(&size_set, f64::from(default_size), cx)
                            },
                        ),
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
                            div()
                                .debug_selector(|| "terminal-color-scheme".into())
                                .child(
                                    Select::new(&scheme_select)
                                        .w(rems(17.5))
                                        .menu_width(rems(20.))
                                        .menu_max_h(rems(22.5)),
                                )
                        })
                        .on_reset(
                            move |cx| color_scheme_is_dirty(&dirty_owner, cx),
                            move |window, cx| {
                                reset_color_scheme(&reset_owner, &reset_select, window, cx)
                            },
                        ),
                    )
                    .description("Terminal color schemes."),
                ),
        )
}

impl Condr {
    pub(in crate::app) fn set_fps_monitor(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.fps_monitor == enabled {
            return;
        }
        self.fps_monitor = enabled;
        self.save_fps_monitor(cx);
        cx.notify();
        cx.refresh_windows();
    }

    pub(in crate::app) fn set_appearance(
        &mut self,
        appearance: Appearance,
        cx: &mut Context<Self>,
    ) {
        if self.appearance == appearance {
            return;
        }
        self.appearance = appearance;
        self.save_appearance(cx);
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

    /// Stores the normalized font, so this value, the theme and the config file
    /// never disagree; the half-typed state lives in the Settings window. Saving is
    /// debounced so a burst of keystrokes rewrites the file once.
    pub(in crate::app) fn set_terminal_font(&mut self, font: TerminalFont, cx: &mut Context<Self>) {
        let font = font.normalized();
        if self.terminal_font == font {
            return;
        }
        self.terminal_font = font;
        apply_terminal_font(&self.terminal_font, cx);
        self._font_save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FONT_SAVE_DEBOUNCE).await;
            // Notify so a save error reaches the UI without waiting for another event.
            let _ = this.update(cx, |this, cx| {
                this.save_terminal_font(cx);
                cx.notify();
            });
        }));
    }

    pub(in crate::app) fn set_terminal_color_scheme(
        &mut self,
        name: SharedString,
        cx: &mut Context<Self>,
    ) {
        if self.terminal_color_scheme == name {
            return;
        }
        self.terminal_color_scheme = name;
        self.save_terminal_color_scheme(cx);
        apply_terminal_color_scheme(&self.terminal_color_scheme, cx);
    }
}
