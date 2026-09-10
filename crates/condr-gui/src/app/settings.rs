mod appearance;
mod server;
mod shortcuts;

use super::*;
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::tab::{Tab, TabBar};
use shortcuts::shortcuts_page;

pub(super) use appearance::*;
pub(super) use server::*;

// Sized in rems so the window zooms with the base font, resolved against the main
// window's rem size when it opens.
const SETTINGS_WINDOW_WIDTH: Rems = rems(54.);
const SETTINGS_WINDOW_HEIGHT: Rems = rems(34.);
/// The page list on the left; two short page names need less than the default.
const SETTINGS_SIDEBAR_WIDTH: Pixels = px(200.);
// Wide enough for the page sidebar plus a label beside the Colors select.
const SETTINGS_WINDOW_MIN_WIDTH: Rems = rems(40.);
const SETTINGS_WINDOW_MIN_HEIGHT: Rems = rems(20.);

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

impl Condr {
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
                ..crate::assets::window_options()
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
    /// The Listen address as typed; see `set_server_listen`.
    pub(super) listen_draft: SharedString,
    /// The Listen address field. Typing only moves the draft; leaving the field or
    /// pressing Enter saves it, so a half-typed address never reaches the Server.
    pub(super) listen_input: Entity<InputState>,
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
        let listen_draft = connection_listen(&owner, selected_server, cx);
        let listen_input =
            cx.new(|cx| InputState::new(window, cx).default_value(listen_draft.clone()));
        cx.subscribe(
            &listen_input,
            |this, input, event: &InputEvent, cx| match event {
                InputEvent::Change => {
                    this.listen_draft = input.read(cx).value();
                    cx.notify();
                }
                InputEvent::Blur | InputEvent::PressEnter { .. } => this.commit_listen(cx),
                InputEvent::Focus => {}
            },
        )
        .detach();
        // Connection status and admin replies live on `Condr`; the Server pages show them.
        if let Some(owner) = owner.upgrade() {
            cx.observe(&owner, |_, _, cx| cx.notify()).detach();
        }
        let _ = owner.update(cx, |owner, _| {
            owner.request_agent_hooks(selected_server);
            owner.request_server_admin(selected_server);
        });
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
            listen_draft,
            listen_input,
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
        self.listen_draft = connection_listen(&self.owner, key, cx);
        let _ = self.owner.update(cx, |owner, _| {
            owner.request_agent_hooks(key);
            owner.request_server_admin(key);
        });
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
        // Root does not draw dialogs itself; without this the Restart and Revoke
        // confirmations open invisibly.
        let dialog_layer = Root::render_dialog_layer(window, cx);
        let settings = cx.entity();
        let tab = self.tab;
        let content = match tab {
            SettingsTab::Application => Settings::new("condr-settings-application")
                .sidebar_width(SETTINGS_SIDEBAR_WIDTH)
                .page(appearance_page(&self.owner, &settings, &self.color_scheme))
                .page(shortcuts_page())
                .page(developer_page(&self.owner))
                .page(licenses_page(&self.licenses)),
            SettingsTab::Server => Settings::new("condr-settings-server")
                .sidebar_width(SETTINGS_SIDEBAR_WIDTH)
                .page(server_terminal_page(&settings))
                .page(server_network_page(&settings))
                .page(server_clients_page(&settings))
                .page(agents_page(&settings, self.connection_hooks(cx))),
        };
        let tabs = TabBar::new("condr-settings-tabs")
            .underline()
            .selected_index(if tab == SettingsTab::Server { 1 } else { 0 })
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
            .prefix(div().w_3())
            .children([Tab::new().label("Application"), Tab::new().label("Server")])
            .when(tab == SettingsTab::Server, |this| {
                this.suffix(
                    div()
                        .debug_selector(|| "settings-server".into())
                        .flex_none()
                        .w(rems(14.))
                        .pr_3()
                        .py_1()
                        .child(Select::new(&self.server_select).small()),
                )
            });
        // The dialog layer sits beside the page, not inside it, so Escape in a dialog
        // closes the dialog and not the window.
        div().size_full().relative().children(dialog_layer).child(
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
                .child(title_bar(SETTINGS_WINDOW_TITLE, cx))
                .child(tabs)
                .child(div().flex_1().min_h_0().child(content)),
        )
    }
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

impl SettingsWindow {
    /// The hooks rows for the selected Server, and why the last request failed if it
    /// did. Read from `self`: this runs inside render, where the entity cannot be read
    /// through its handle.
    fn connection_hooks(&self, cx: &App) -> (Vec<HooksReport>, Option<String>) {
        self.owner
            .upgrade()
            .and_then(|owner| {
                owner
                    .read(cx)
                    .connections
                    .iter()
                    .find(|connection| connection.key == self.selected_server)
                    .map(|connection| (connection.hooks.clone(), connection.hooks_error.clone()))
            })
            .unwrap_or_default()
    }
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
            for (_, rows) in super::shortcuts::SHORTCUTS {
                for (label, action) in *rows {
                    assert!(
                        gpui_kit::component::kbd::Kbd::binding_for_action(
                            *action,
                            Some(super::shortcuts::SHORTCUT_CONTEXT),
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
