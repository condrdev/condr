use super::*;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::tab::{Tab, TabBar};

// Sized in rems so the window zooms with the base font, resolved against the main
// window's rem size when it opens.
const SETTINGS_WINDOW_WIDTH: Rems = rems(54.);
const SETTINGS_WINDOW_HEIGHT: Rems = rems(34.);
/// The page list on the left; two short page names need less than the default.
const SETTINGS_SIDEBAR_WIDTH: Pixels = px(200.);
// Wide enough for the page sidebar plus a label beside the Colors select.
const SETTINGS_WINDOW_MIN_WIDTH: Rems = rems(40.);
const SETTINGS_WINDOW_MIN_HEIGHT: Rems = rems(20.);
/// How long after the last font keystroke the config file is written.
const FONT_SAVE_DEBOUNCE: Duration = Duration::from_millis(300);
/// Shown in the drawn title bar and set as the OS title for the taskbar.
const SETTINGS_WINDOW_TITLE: &str = "Condr — Settings";

/// Centered over the main window and kept inside that window's display, minus the
/// taskbar or Dock: a main window on a secondary screen gets its Settings there, and
/// a small screen gets a smaller Settings window rather than one hanging off the edge.
fn settings_window_bounds(window: &Window, size: Size<Pixels>, cx: &App) -> Bounds<Pixels> {
    let main = window.bounds();
    let screen = window.display(cx).map(|display| display.visible_bounds());
    let size = match &screen {
        Some(screen) => gpui_kit::size(
            size.width.min(screen.size.width),
            size.height.min(screen.size.height),
        ),
        None => size,
    };
    let mut origin = point(
        main.origin.x + (main.size.width - size.width) / 2.,
        main.origin.y + (main.size.height - size.height) / 2.,
    );
    if let Some(screen) = screen {
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

    /// Whole pixels within bounds: the stepper moves by one and shows integers, so a
    /// hand-edited `13.5` becomes 14 rather than a value the UI cannot display.
    pub(super) fn clamp_size(size: f32) -> f32 {
        if size.is_finite() {
            size.round().clamp(Self::MIN_SIZE, Self::MAX_SIZE)
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
    pub(super) fn set_fps_monitor(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.fps_monitor == enabled {
            return;
        }
        self.fps_monitor = enabled;
        self.save_fps_monitor(cx);
        cx.notify();
        cx.refresh_windows();
    }

    pub(super) fn set_appearance(&mut self, appearance: Appearance, cx: &mut Context<Self>) {
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
    pub(super) fn set_terminal_font(&mut self, font: TerminalFont, cx: &mut Context<Self>) {
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

    pub(super) fn set_terminal_color_scheme(&mut self, name: SharedString, cx: &mut Context<Self>) {
        if self.terminal_color_scheme == name {
            return;
        }
        self.terminal_color_scheme = name;
        self.save_terminal_color_scheme(cx);
        apply_terminal_color_scheme(&self.terminal_color_scheme, cx);
    }

    /// Asks a Server to store a new shell preference. The stored value comes back as a
    /// `ServerSettingsChanged` event; nothing is assumed locally.
    pub(super) fn set_server_shell(
        &mut self,
        key: ConnectionKey,
        shell: &str,
        cx: &mut Context<Self>,
    ) {
        // Debounced like the font: the Server persists every value it receives, so a
        // half-typed path must not reach config.toml or the next new terminal. A value
        // for another Server still goes out before this one replaces it.
        if self
            .pending_shell
            .as_ref()
            .is_some_and(|(pending_key, _)| *pending_key != key)
        {
            self.flush_server_shell();
        }
        self.pending_shell = Some((key, shell.to_owned()));
        self._shell_save = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(FONT_SAVE_DEBOUNCE).await;
            let _ = this.update(cx, |this, _| this.flush_server_shell());
        }));
    }

    /// Sends the debounced Shell value now, if one is waiting.
    pub(super) fn flush_server_shell(&mut self) {
        if let Some((key, shell)) = self.pending_shell.take() {
            self.send_server_shell(key, &shell);
        }
    }

    fn send_server_shell(&mut self, key: ConnectionKey, shell: &str) {
        let Some(connection) = self
            .connections
            .iter_mut()
            .find(|connection| connection.key == key)
        else {
            return;
        };
        let Some(server_id) = connection.server_id else {
            return;
        };
        connection.send(ClientMessage::SetServerSettings {
            server_id,
            shell: shell.to_owned(),
        });
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
        // The bounds are in this display's coordinates; without the id, Windows
        // would place them on the primary monitor.
        let display_id = window.display(cx).map(|display| display.id());
        let main_window = self.window_handle;
        let owner = cx.weak_entity();
        // Opening a window needs the App without this entity on the stack.
        cx.defer(move |cx| {
            let options = WindowOptions {
                display_id,
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(min_size),
                ..TitleBar::window_options()
            };
            let view_owner = owner.clone();
            let opened = cx.open_window(options, |window, cx| {
                window.set_window_title(SETTINGS_WINDOW_TITLE);
                let view = cx.new(|cx| SettingsWindow::new(view_owner.clone(), window, cx));
                view.read(cx).focus_handle.clone().focus(window, cx);
                let _ =
                    view_owner.update(cx, |this, _| this.settings_view = Some(view.downgrade()));
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
    pub(super) color_scheme: Entity<ColorSchemeSelect>,
    /// The font as typed in this window. `Condr` only ever holds the normalized
    /// form, so a half-edited value never reaches the theme or the config file, and
    /// reopening Settings starts from what is actually in use.
    pub(super) font_draft: TerminalFont,
    /// Which Server the Server page edits; starts at the active connection.
    pub(super) selected_server: ConnectionKey,
    /// The Shell field as typed. The Server stores the trimmed value and echoes it
    /// through its settings; the field is not rewritten under the user meanwhile.
    pub(super) shell_draft: SharedString,
    /// Which tab is showing: this Client's settings or one Server's.
    pub(super) tab: SettingsTab,
    /// The Server picker in the tab bar. Its items mirror `server_keys` by index,
    /// refreshed on render when the connection list changes.
    server_select: Entity<ServerSelect>,
    server_keys: Vec<ConnectionKey>,
    server_labels: Vec<SharedString>,
    /// The Licenses page text, in a read-only editor because it is far too long
    /// for a plain text element.
    licenses: Entity<EditorState>,
}

/// The two halves of Settings: this Client's own preferences and one Server's.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum SettingsTab {
    #[default]
    Application,
    Server,
}

type ServerSelect = SelectState<SearchableVec<SharedString>>;

/// Connection keys and labels in sidebar order, for the Server picker.
/// The picker identifies a Server by its label, so two Servers sharing a name get the
/// endpoint appended to stay distinguishable.
fn server_choices(owner: &WeakEntity<Condr>, cx: &App) -> (Vec<ConnectionKey>, Vec<SharedString>) {
    owner
        .upgrade()
        .map(|owner| {
            let connections = &owner.read(cx).connections;
            connections
                .iter()
                .map(|connection| {
                    let duplicated = connections
                        .iter()
                        .filter(|other| other.label == connection.label)
                        .count()
                        > 1;
                    let label = if duplicated {
                        format!("{} ({})", connection.label, connection.endpoint)
                    } else {
                        connection.label.clone()
                    };
                    (connection.key, SharedString::from(label))
                })
                .unzip()
        })
        .unwrap_or_default()
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
                .soft_wrap(false)
                .line_number(false)
                // License texts are indented prose, not code: no guides, no fold gutter.
                .indent_guides(false)
                .folding(false)
                .searchable(true)
        });
        let font_draft = owner
            .upgrade()
            .map(|owner| owner.read(cx).terminal_font.clone())
            .unwrap_or_default();
        let selected_server = owner
            .upgrade()
            .map(|owner| owner.read(cx).active_connection)
            .unwrap_or_default();
        let shell_draft = connection_shell(&owner, selected_server, cx);
        let (server_keys, server_labels) = server_choices(&owner, cx);
        let selected = server_keys
            .iter()
            .position(|key| *key == selected_server)
            .map(IndexPath::new);
        let server_select = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(server_labels.clone()),
                selected,
                window,
                cx,
            )
        });
        cx.subscribe(
            &server_select,
            |this, _, event: &SelectEvent<SearchableVec<SharedString>>, cx| {
                let SelectEvent::Confirm(Some(label)) = event else {
                    return;
                };
                // ponytail: first label match; two Servers with one name pick the first.
                if let Some(key) = this
                    .server_labels
                    .iter()
                    .position(|candidate| candidate == label)
                    .and_then(|index| this.server_keys.get(index).copied())
                {
                    this.select_server(key, cx);
                }
            },
        )
        .detach();
        Self {
            owner,
            focus_handle: cx.focus_handle(),
            color_scheme,
            font_draft,
            selected_server,
            shell_draft,
            tab: SettingsTab::default(),
            server_select,
            server_keys,
            server_labels,
            licenses,
        }
    }

    /// Switches the Server tab to another connection and reloads its shell.
    fn select_server(&mut self, key: ConnectionKey, cx: &mut Context<Self>) {
        self.selected_server = key;
        self.shell_draft = connection_shell(&self.owner, key, cx);
        cx.notify();
    }

    /// Keeps the picker's items in step with the connections, which can change while
    /// Settings is open.
    fn refresh_server_choices(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (keys, labels) = server_choices(&self.owner, cx);
        if labels == self.server_labels && keys == self.server_keys {
            return;
        }
        // A removed Server falls back to the first one, shell draft included.
        if !keys.contains(&self.selected_server)
            && let Some(&first) = keys.first()
        {
            self.select_server(first, cx);
        }
        let selected = keys
            .iter()
            .position(|key| *key == self.selected_server)
            .map(IndexPath::new);
        self.server_keys = keys;
        self.server_labels = labels.clone();
        self.server_select.update(cx, |select, cx| {
            select.set_items(SearchableVec::new(labels), window, cx);
            select.set_selected_index(selected, window, cx);
        });
    }

    /// Pushes the draft to `Condr`, which normalizes and applies it.
    fn commit_font(&self, cx: &mut Context<Self>) {
        let font = self.font_draft.clone();
        let _ = self
            .owner
            .update(cx, move |owner, cx| owner.set_terminal_font(font, cx));
    }
}

fn licenses_page(licenses: &Entity<EditorState>) -> SettingPage {
    let licenses = licenses.clone();
    SettingPage::new("Licenses").icon(IconName::BookOpen).group(
        SettingGroup::new().item(
            SettingItem::render(move |_, _, cx| {
                // Plain text: no border, background or line numbers. It stays an
                // Editor only because a 900 KB text needs virtualized rendering.
                // Prose, not code: the UI font rather than the terminal font.
                Editor::new(&licenses)
                    .readonly(true)
                    .appearance(false)
                    .bordered(false)
                    .font_family(cx.theme().font_family.clone())
                    .text_size(cx.theme().font_size)
                    .h(rems(26.))
            })
            // Settings search only matches custom items by keyword.
            .keywords([
                "licenses",
                "license",
                "third-party",
                "open source",
                "notices",
                "attribution",
            ]),
        ),
    )
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.refresh_server_choices(window, cx);
        let settings = cx.entity();
        let tab = self.tab;
        // One `Settings` per tab, each with its own id so their page selection and
        // search do not bleed into each other.
        let content = match tab {
            SettingsTab::Application => Settings::new("condr-settings-application")
                // Matches the main window's sidebar.
                .sidebar_width(SETTINGS_SIDEBAR_WIDTH)
                .page(appearance_page(&self.owner, &settings, &self.color_scheme))
                .page(shortcuts_page())
                .page(developer_page(&self.owner))
                .page(licenses_page(&self.licenses)),
            SettingsTab::Server => Settings::new("condr-settings-server")
                .sidebar_width(SETTINGS_SIDEBAR_WIDTH)
                .page(server_page(&settings)),
        };
        let tabs = TabBar::new("condr-settings-tabs")
            .underline()
            .selected_index(match tab {
                SettingsTab::Application => 0,
                SettingsTab::Server => 1,
            })
            .on_click(move |index, _, cx| {
                let tab = if *index == 1 {
                    SettingsTab::Server
                } else {
                    SettingsTab::Application
                };
                settings.update(cx, |this, cx| {
                    if this.tab != tab {
                        this.tab = tab;
                        cx.notify();
                    }
                });
            })
            // The tabs start flush left otherwise; match the page sidebar's inset.
            .prefix(div().w_3())
            .children([Tab::new().label("Application"), Tab::new().label("Server")])
            // The picker sits beside the tabs, so the whole Server tab reads as "this
            // Server's settings". The Select fills its container, so the wrapper sets
            // the width.
            .when(tab == SettingsTab::Server, |this| {
                this.suffix(
                    div()
                        .debug_selector(|| "settings-server".into())
                        .flex_none()
                        .w(rems(11.))
                        .pr_3()
                        .py_1()
                        .child(Select::new(&self.server_select).small()),
                )
            });
        v_flex()
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
            .child(TitleBar::new().child(SETTINGS_WINDOW_TITLE))
            .child(tabs)
            .child(div().flex_1().min_h_0().child(content))
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

// The field callbacks below are named so tests can drive them the way the widgets
// do; the widgets themselves belong to GPUI Kit and expose no test hooks.

/// What the Font field shows: the value as typed in this window.
pub(super) fn terminal_font_family(settings: &Entity<SettingsWindow>, cx: &App) -> SharedString {
    settings.read(cx).font_draft.family.clone()
}

/// What the Font field does on every change.
pub(super) fn select_terminal_font_family(
    settings: &Entity<SettingsWindow>,
    family: SharedString,
    cx: &mut App,
) {
    settings.update(cx, |this, cx| {
        this.font_draft.family = family;
        this.commit_font(cx);
    });
}

/// What the Font size field shows.
pub(super) fn terminal_font_size(settings: &Entity<SettingsWindow>, cx: &App) -> f64 {
    f64::from(settings.read(cx).font_draft.size)
}

/// What the Font size field does on every change.
pub(super) fn select_terminal_font_size(
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
pub(super) fn step_terminal_font_size(settings: &Entity<SettingsWindow>, delta: f32, cx: &mut App) {
    settings.update(cx, |this, cx| {
        this.font_draft.size = TerminalFont::clamp_size(this.font_draft.size + delta);
        this.commit_font(cx);
    });
}

/// The shell a Server currently stores, as its Bootstrap or last event reported it.
fn connection_shell(owner: &WeakEntity<Condr>, key: ConnectionKey, cx: &App) -> SharedString {
    owner
        .upgrade()
        .and_then(|owner| {
            owner
                .read(cx)
                .connections
                .iter()
                .find(|connection| connection.key == key)
                .map(|connection| connection.settings.shell.clone().into())
        })
        .unwrap_or_default()
}

/// What picking a Server in the tab bar does; tests call it without the widget.
#[cfg(all(test, feature = "test-support"))]
pub(super) fn select_settings_server(
    settings: &Entity<SettingsWindow>,
    key: ConnectionKey,
    cx: &mut App,
) {
    settings.update(cx, |this, cx| this.select_server(key, cx));
}

/// What the Shell field shows.
pub(super) fn server_shell(settings: &Entity<SettingsWindow>, cx: &App) -> SharedString {
    settings.read(cx).shell_draft.clone()
}

/// What the Shell field does on every change.
pub(super) fn select_server_shell(
    settings: &Entity<SettingsWindow>,
    shell: SharedString,
    cx: &mut App,
) {
    settings.update(cx, |this, cx| {
        this.shell_draft = shell.clone();
        let key = this.selected_server;
        let _ = this
            .owner
            .update(cx, |owner, cx| owner.set_server_shell(key, &shell, cx));
    });
}

/// Whether Reset All has anything to do for Colors.
pub(super) fn color_scheme_is_dirty(owner: &WeakEntity<Condr>, cx: &App) -> bool {
    owner
        .upgrade()
        .is_some_and(|owner| !owner.read(cx).terminal_color_scheme.is_empty())
}

/// What Reset All does for Colors: back to the built-in palette, in both the
/// preference and the select widget.
pub(super) fn reset_color_scheme(
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
fn appearance_page(
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
    let family_get = settings.clone();
    let family_set = settings.clone();
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
                        SettingField::input(
                            move |cx| terminal_font_family(&family_get, cx),
                            move |value: SharedString, cx| {
                                select_terminal_font_family(&family_set, value, cx)
                            },
                        )
                        .default_value(default_font.family.clone()),
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

fn developer_page(owner: &WeakEntity<Condr>) -> SettingPage {
    let value_owner = owner.clone();
    let set_owner = owner.clone();
    SettingPage::new("Developer")
        .icon(IconName::Inspector)
        .group(
            SettingGroup::new().title("Diagnostics").item(
                SettingItem::new(
                    "FPS monitor",
                    SettingField::switch(
                        move |cx| {
                            value_owner
                                .upgrade()
                                .is_some_and(|owner| owner.read(cx).fps_monitor)
                        },
                        move |enabled, cx| {
                            let _ = set_owner
                                .update(cx, |owner, cx| owner.set_fps_monitor(enabled, cx));
                        },
                    )
                    .default_value(false),
                )
                .description("Show GPUI frame rate and resource telemetry."),
            ),
        )
}

/// A labelled action, as one row of the Shortcuts page.
type Shortcut = (&'static str, &'static dyn Action);

/// The fixed shortcuts, grouped as the page shows them. The keys themselves are read
/// from the keymap at render time, so this list can never disagree with `bind_keys`.
pub(super) const SHORTCUTS: &[(&str, &[Shortcut])] = &[
    ("Application", &[("Open Settings", &OpenSettings)]),
    (
        "Tabs",
        &[
            ("New tab", &NewTab),
            ("Next tab", &NextTab),
            ("Previous tab", &PreviousTab),
        ],
    ),
    (
        "Panes",
        &[
            ("Split right", &SplitRight),
            ("Split down", &SplitDown),
            ("Close pane", &ClosePane),
            ("Toggle zoom", &ToggleZoom),
            ("Focus left", &FocusLeft),
            ("Focus right", &FocusRight),
            ("Focus up", &FocusUp),
            ("Focus down", &FocusDown),
            ("Resize left", &ResizeLeft),
            ("Resize right", &ResizeRight),
            ("Resize up", &ResizeUp),
            ("Resize down", &ResizeDown),
        ],
    ),
];

/// The key context `bind_keys` registers the shortcuts under.
pub(super) const SHORTCUT_CONTEXT: &str = "Condr";

/// Read-only: shortcuts are fixed per platform, so this page only shows them.
fn shortcuts_page() -> SettingPage {
    SettingPage::new("Shortcuts")
        .icon(IconName::LayoutDashboard)
        .groups(SHORTCUTS.iter().map(|(title, rows)| {
            rows.iter().fold(
                SettingGroup::new().title(*title),
                |group, (label, action)| {
                    group.item(SettingItem::new(
                        *label,
                        SettingField::render(move |_, window, _| {
                            div().children(
                                Kbd::binding_for_action(*action, Some(SHORTCUT_CONTEXT), window)
                                    .map(|kbd| kbd.outline()),
                            )
                        }),
                    ))
                },
            )
        }))
}

/// Preferences a Server owns, edited for one connection at a time. Only the shell so
/// far; the Server picker sits in the tab bar.
fn server_page(settings: &Entity<SettingsWindow>) -> SettingPage {
    let shell_get = settings.clone();
    let shell_set = settings.clone();
    SettingPage::new("Server")
        .icon(IconName::Cpu)
        .default_open(true)
        .group(
            SettingGroup::new().title("Terminal").item(
                SettingItem::new(
                    "Shell",
                    SettingField::input(
                        move |cx| server_shell(&shell_get, cx),
                        move |value: SharedString, cx| select_server_shell(&shell_set, value, cx),
                    )
                    .default_value(""),
                )
                .description("Empty uses the system default."),
            ),
        )
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: it would glob in `gpui_kit::test` and shadow the built-in attribute.
    #[cfg(feature = "test-support")]
    use gpui_kit::component::ActiveTheme as _;

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
        assert_eq!(TerminalFont::clamp_size(13.5), 14.);
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
    #[gpui_kit::test]
    fn the_terminal_font_survives_an_appearance_change(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            let font = TerminalFont {
                family: "Cascadia Mono".into(),
                size: 17.,
            };
            apply_terminal_font(&font, cx);
            apply_appearance(Appearance::Dark, None, cx);
            apply_appearance(Appearance::Light, None, cx);
            assert_eq!(cx.theme().mono_font_family.as_ref(), "Cascadia Mono");
            assert_eq!(cx.theme().mono_font_size, gpui_kit::px(17.));
        });
    }

    /// A listed action without a binding would render an empty row.
    #[cfg(feature = "test-support")]
    #[gpui_kit::test]
    fn every_listed_shortcut_has_a_binding_on_this_platform(cx: &mut gpui_kit::TestAppContext) {
        cx.update(super::super::startup::bind_keys);
        let window = cx.add_empty_window();
        window.update(|window, _| {
            for (_, rows) in super::SHORTCUTS {
                for (label, action) in *rows {
                    assert!(
                        gpui_kit::component::kbd::Kbd::binding_for_action(
                            *action,
                            Some(super::SHORTCUT_CONTEXT),
                            window
                        )
                        .is_some(),
                        "{label} has no binding"
                    );
                }
            }
        });
    }

    #[cfg(feature = "test-support")]
    #[gpui_kit::test]
    fn applying_an_appearance_pins_the_theme_mode(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            gpui_kit::init(cx);

            apply_appearance(Appearance::Dark, None, cx);
            assert!(cx.theme().is_dark());

            apply_appearance(Appearance::Light, None, cx);
            assert!(!cx.theme().is_dark());
        });
    }
}
