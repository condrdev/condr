use super::*;

#[test]
fn the_settings_button_opens_a_separate_window_that_applies_a_theme_mode() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        gpui_kit::init(cx);
        super::super::super::startup::bind_keys(cx);
    });
    let (view, window, _server) = connected_condr(&mut cx);
    let main_window = window.update(|window, _| window.window_handle());

    let settings_button = window
        .debug_bounds("open-settings")
        .expect("the sidebar should render a Settings button");
    window.simulate_click(settings_button.center(), Modifiers::default());
    window.run_until_parked();
    let settings_handle = window
        .windows()
        .into_iter()
        .find(|handle| *handle != main_window)
        .expect("the Settings button should open a second window");
    let settings = VisualTestContext::from_window(settings_handle, window).into_mut();
    settings.update(|window, cx| _ = window.draw(cx));
    assert!(
        settings.debug_bounds("settings-content").is_some(),
        "the Settings window should render its content"
    );

    // Opening again re-activates the existing window instead of adding another.
    window.simulate_click(settings_button.center(), Modifiers::default());
    window.run_until_parked();
    assert_eq!(window.windows().len(), 2);

    window.update(|window, cx| {
        view.update(cx, |this, cx| this.set_appearance(Appearance::Dark, cx));
        _ = window.draw(cx);
    });
    window.run_until_parked();
    assert!(
        window.update(|_, cx| cx.theme().is_dark()),
        "choosing Dark should switch the theme mode"
    );
    assert_eq!(
        window.windows().len(),
        2,
        "switching the theme must keep the Settings window open"
    );

    settings.update(|window, _| window.remove_window());
    window.run_until_parked();
    assert_eq!(window.windows().len(), 1);

    // Closing the main window takes a reopened Settings window with it.
    window.simulate_click(settings_button.center(), Modifiers::default());
    window.run_until_parked();
    assert_eq!(window.windows().len(), 2);
    window.update(|window, _| window.remove_window());
    window.run_until_parked();
    assert!(
        cx.windows().is_empty(),
        "closing the main window must close Settings too"
    );
}

#[test]
fn the_terminal_settings_controls_drive_the_preferences_and_reset() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        gpui_kit::init(cx);
        super::super::super::startup::bind_keys(cx);
    });
    let (view, window, _server) = connected_condr(&mut cx);
    let owner = view.downgrade();
    let main_window = window.update(|window, _| window.window_handle());

    let settings_button = window.debug_bounds("open-settings").unwrap();
    window.simulate_click(settings_button.center(), Modifiers::default());
    window.run_until_parked();
    let settings_handle = window
        .windows()
        .into_iter()
        .find(|handle| *handle != main_window)
        .expect("Settings should open in its own window");
    let settings = VisualTestContext::from_window(settings_handle, window).into_mut();
    settings.update(|window, cx| _ = window.draw(cx));
    let settings_view = settings.read(|app| {
        view.read(app)
            .settings_view
            .as_ref()
            .and_then(|view| view.upgrade())
            .expect("the Settings window view is recorded")
    });
    let select_state = settings.read(|app| settings_view.read(app).color_scheme.clone());

    // Colors: the real Select, opened by click, filtered by typing, confirmed by Enter.
    let select = settings
        .debug_bounds("terminal-color-scheme")
        .expect("the Colors select should render");
    settings.simulate_click(select.center(), Modifiers::default());
    settings.run_until_parked();
    settings.simulate_input("Kanagawa Dragon");
    settings.simulate_keystrokes("enter");
    settings.run_until_parked();
    assert_eq!(
        window.read(|app| view.read(app).terminal_color_scheme.clone()),
        "Kanagawa Dragon",
        "confirming a scheme in the select must store it"
    );
    let expected = crate::color_scheme::palette("Kanagawa Dragon").unwrap();
    assert_eq!(
        window.read(|app| app.global::<TerminalPalette>().background),
        expected.background,
        "the chosen scheme must reach the terminal palette"
    );
    assert!(window.read(|app| color_scheme_is_dirty(&owner, app)));

    // Reset All for Colors: preference and widget both return to Default.
    settings.update(|window, cx| reset_color_scheme(&owner, &select_state, window, cx));
    settings.run_until_parked();
    assert_eq!(
        window.read(|app| view.read(app).terminal_color_scheme.clone()),
        ""
    );
    assert!(!window.read(|app| color_scheme_is_dirty(&owner, app)));
    assert_eq!(
        window.read(|app| select_state.read(app).selected_value().cloned()),
        Some("Default".into()),
        "the select must show Default after a reset"
    );
    assert_eq!(
        window.read(|app| app.global::<TerminalPalette>().background),
        TerminalPalette::default().background
    );

    // Font: the field keeps what was typed; the theme gets the normalized value.
    window.update(|_, cx| select_terminal_font_family(&settings_view, "Cascadia Mono".into(), cx));
    assert_eq!(
        window.read(|app| terminal_font_family(&settings_view, app)),
        "Cascadia Mono"
    );
    assert_eq!(
        window.read(|app| app.theme().mono_font_family.clone()),
        "Cascadia Mono"
    );
    window.update(|_, cx| select_terminal_font_family(&settings_view, "".into(), cx));
    assert_eq!(
        window.read(|app| terminal_font_family(&settings_view, app)),
        "",
        "a half-edited field must not be rewritten under the user"
    );
    assert_eq!(
        window.read(|app| app.theme().mono_font_family.clone()),
        TerminalFont::default().family,
        "an empty family falls back to the default font"
    );

    // Shell: the Server page edits the selected Server; the Server stores the value and
    // reports it back, so the connection's settings follow the field.
    assert_eq!(
        window.read(|app| settings_view.read(app).selected_server),
        1,
        "the Server tab starts on the active connection"
    );
    window.update(|_, cx| select_settings_server(&settings_view, 1, cx));
    assert_eq!(window.read(|app| server_shell(&settings_view, app)), "");
    window.update(|_, cx| select_server_shell(&settings_view, " nu ".into(), cx));
    assert_eq!(
        window.read(|app| server_shell(&settings_view, app)),
        " nu ",
        "the field keeps what was typed"
    );
    assert!(
        wait_until_event_driven(window, |window| {
            window.read(|app| {
                view.read(app)
                    .connection(1)
                    .is_some_and(|connection| connection.settings.shell == "nu")
            })
        }),
        "the Server must store the trimmed shell and publish it"
    );
    window.update(|_, cx| select_server_shell(&settings_view, "".into(), cx));
    assert!(wait_until_event_driven(window, |window| {
        window.read(|app| {
            view.read(app)
                .connection(1)
                .is_some_and(|connection| connection.settings.shell.is_empty())
        })
    }));

    // Font size: the real stepper buttons, one pixel per click, clamped to the bounds.
    let default_size = f64::from(TerminalFont::default().size);
    settings.update(|window, cx| _ = window.draw(cx));
    let decrease = settings
        .debug_bounds("terminal-font-size-decrease")
        .expect("the Font size stepper should render");
    settings.simulate_click(decrease.center(), Modifiers::default());
    settings.run_until_parked();
    assert_eq!(
        window.read(|app| terminal_font_size(&settings_view, app)),
        default_size - 1.
    );
    assert_eq!(
        window.read(|app| app.theme().mono_font_size),
        px(TerminalFont::default().size - 1.)
    );
    settings.update(|window, cx| _ = window.draw(cx));
    let increase = settings
        .debug_bounds("terminal-font-size-increase")
        .unwrap();
    settings.simulate_click(increase.center(), Modifiers::default());
    settings.simulate_click(increase.center(), Modifiers::default());
    settings.run_until_parked();
    assert_eq!(
        window.read(|app| app.theme().mono_font_size),
        px(TerminalFont::default().size + 1.)
    );
    window.update(|_, cx| step_terminal_font_size(&settings_view, -100., cx));
    assert_eq!(
        window.read(|app| app.theme().mono_font_size),
        px(TerminalFont::MIN_SIZE),
        "stepping never leaves the allowed range"
    );
    window.update(|_, cx| select_terminal_font_size(&settings_view, default_size, cx));
    assert_eq!(
        window.read(|app| app.theme().mono_font_size),
        px(TerminalFont::default().size)
    );

    // Agents: opening Settings asked the Server for every agent's hooks state, and the
    // Server tab renders those rows without reading the window entity re-entrantly.
    assert!(
        wait_until_event_driven(window, |window| {
            window.read(|app| {
                view.read(app).connection(1).is_some_and(|connection| {
                    connection.hooks.len() == condr_core::AgentKind::ALL.len()
                })
            })
        }),
        "every agent's hooks state must arrive from the Server"
    );
    window.update(|_, cx| select_settings_tab(&settings_view, SettingsTab::Server, cx));
    settings.update(|window, cx| _ = window.draw(cx));
    settings.run_until_parked();

    settings.update(|window, _| window.remove_window());
    window.run_until_parked();
}
#[test]
fn chosen_appearance_persists_and_survives_gui_restart() {
    let _serial_guard = acquire_visual_test_lock();
    let directory = TestDirectory::new("client-appearance");
    let config_path = directory.0.join("config.toml");
    let (server, endpoint) = start_server();

    {
        let mut cx = TestAppContext::single();
        cx.update(gpui_kit::init);
        let initial = ClientConnection::connect(&endpoint, "condr-test").unwrap();
        let view_holder = Rc::new(RefCell::new(None));
        let view_holder_for_window = view_holder.clone();
        let (_root, window) = cx.add_window_view(|window, cx| {
            let view = cx.new(|cx| {
                Condr::new(
                    endpoint.clone(),
                    Some(Ok(initial)),
                    config::LoadedConfig::read(Some(config_path.clone())),
                    gui_state::LoadedState::default(),
                    window,
                    cx,
                )
            });
            view_holder_for_window.borrow_mut().replace(view.clone());
            Root::new(view, window, cx)
        });
        let view = view_holder.borrow_mut().take().unwrap();
        assert!(!window.update(|_, cx| cx.theme().is_dark()));

        let config_lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(directory.0.join("config.toml.lock"))
            .unwrap();
        config_lock.lock().unwrap();
        // UI callbacks must return while another process owns the config transaction.
        for appearance in [Appearance::Dark, Appearance::Light, Appearance::Dark] {
            window.update(|_, cx| {
                view.update(cx, |this, cx| this.set_appearance(appearance, cx));
            });
        }
        window.update(|_, cx| {
            view.update(cx, |this, cx| {
                this.set_terminal_font(
                    TerminalFont {
                        family: "Cascadia Mono".into(),
                        size: 18.,
                    },
                    cx,
                )
            });
        });
        assert!(window.update(|_, cx| cx.theme().is_dark()));
        assert!(
            window.read(|app| view.read(app).last_error.is_none()),
            "saving the appearance must not report a config error"
        );
        assert!(!config_path.exists());
        drop(config_lock);
        // Quitting before a task or debounce runs must flush the ordered queue.
        cx.quit();
        assert!(
            std::fs::read_to_string(&config_path)
                .unwrap()
                .contains("appearance = \"dark\""),
            "the appearance must reach the client config file"
        );
        assert!(
            std::fs::read_to_string(&config_path)
                .unwrap()
                .contains("font_size = 18.0")
        );
    }

    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    assert!(
        !cx.update(|cx| cx.theme().is_dark()),
        "gpui_kit::init starts every process in Light"
    );
    let initial = ClientConnection::connect(&endpoint, "condr-test").unwrap();
    let view_holder = Rc::new(RefCell::new(None));
    let view_holder_for_window = view_holder.clone();
    let (_root, window) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| {
            Condr::new(
                endpoint,
                Some(Ok(initial)),
                config::LoadedConfig::read(Some(config_path)),
                gui_state::LoadedState::default(),
                window,
                cx,
            )
        });
        view_holder_for_window.borrow_mut().replace(view.clone());
        Root::new(view, window, cx)
    });
    let view = view_holder.borrow_mut().take().unwrap();
    assert!(
        window.update(|_, cx| cx.theme().is_dark()),
        "a restarted GUI must restore the saved appearance"
    );
    assert_eq!(
        window.read(|app| view.read(app).appearance),
        Appearance::Dark
    );
    drop(server);
}

#[test]
fn font_changes_reach_the_config_once_the_debounce_elapses() {
    let _serial_guard = acquire_visual_test_lock();
    let directory = TestDirectory::new("client-font-debounce");
    let config_path = directory.0.join("config.toml");
    let (server, endpoint) = start_server();

    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let initial = ClientConnection::connect(&endpoint, "condr-test").unwrap();
    let view_holder = Rc::new(RefCell::new(None));
    let view_holder_for_window = view_holder.clone();
    let (_root, window) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| {
            Condr::new(
                endpoint.clone(),
                Some(Ok(initial)),
                config::LoadedConfig::read(Some(config_path.clone())),
                gui_state::LoadedState::default(),
                window,
                cx,
            )
        });
        view_holder_for_window.borrow_mut().replace(view.clone());
        Root::new(view, window, cx)
    });
    let view = view_holder.borrow_mut().take().unwrap();

    // Two quick edits: the file is written once, with the last value, and only
    // after the debounce, so typing does not hit the disk per keystroke.
    for family in ["Cascadia", "Cascadia Mono"] {
        window.update(|_, cx| {
            view.update(cx, |this, cx| {
                this.set_terminal_font(
                    TerminalFont {
                        family: family.into(),
                        size: 1.,
                    },
                    cx,
                )
            });
        });
    }
    window.run_until_parked();
    assert!(
        !config_path.exists(),
        "the config must not be written before the debounce elapses"
    );
    window.executor().advance_clock(Duration::from_secs(1));
    window.run_until_parked();
    let saved = std::fs::read_to_string(&config_path).unwrap();
    assert!(
        saved.contains("font_family = \"Cascadia Mono\""),
        "the last font must be saved:\n{saved}"
    );
    assert!(
        !saved.contains("\"Cascadia\"\n"),
        "the intermediate value must never reach the file:\n{saved}"
    );
    assert!(
        saved.contains(&format!("font_size = {:.1}", TerminalFont::MIN_SIZE)),
        "the saved size is the normalized one:\n{saved}"
    );
    drop(server);
}

#[test]
fn the_mode_dropdown_reads_and_writes_the_appearance() {
    let _serial_guard = acquire_visual_test_lock();
    let directory = TestDirectory::new("client-dropdown");
    let config_path = directory.0.join("config.toml");
    let (server, endpoint) = start_server();

    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let initial = ClientConnection::connect(&endpoint, "condr-test").unwrap();
    let view_holder = Rc::new(RefCell::new(None));
    let view_holder_for_window = view_holder.clone();
    let (_root, window) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| {
            Condr::new(
                endpoint.clone(),
                Some(Ok(initial)),
                config::LoadedConfig::read(Some(config_path.clone())),
                gui_state::LoadedState::default(),
                window,
                cx,
            )
        });
        view_holder_for_window.borrow_mut().replace(view.clone());
        Root::new(view, window, cx)
    });
    let view = view_holder.borrow_mut().take().unwrap();
    let owner = view.downgrade();

    assert_eq!(
        window.read(|app| selected_appearance(&owner, app)),
        "system",
        "the dropdown starts on the stored preference"
    );

    window.update(|_, cx| select_appearance(&owner, "dark", cx));
    window.run_until_parked();
    assert_eq!(
        window.read(|app| selected_appearance(&owner, app)),
        "dark",
        "choosing an option must move the dropdown to it"
    );
    assert!(window.update(|_, cx| cx.theme().is_dark()));
    assert!(
        std::fs::read_to_string(&config_path)
            .unwrap()
            .contains("appearance = \"dark\""),
        "choosing an option must reach the client config file"
    );

    // The dropdown hands back whatever string its option carried, so an option that no
    // longer matches an Appearance must land on System rather than on nothing.
    window.update(|_, cx| select_appearance(&owner, "solarized", cx));
    assert_eq!(
        window.read(|app| selected_appearance(&owner, app)),
        "system"
    );
    drop(server);
}

#[test]
fn hooks_reports_replace_their_agents_row_and_errors_clear_on_the_next_report() {
    use condr_core::agent_hooks::{HooksReport, HooksState};
    use condr_core::protocol::{AgentError, AgentResponse};
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let generation = this.connection(1).unwrap().connect_generation;
            let mut deliver = |result| {
                this.handle_incoming(
                    1,
                    generation,
                    Incoming::Message(ServerMessage::AgentResult { result }),
                    cx,
                );
            };
            let report = |state, warning: Option<&str>| HooksReport {
                agent: AgentKind::Codex,
                path: std::path::PathBuf::from("hooks.json"),
                state,
                note: None,
                warning: warning.map(str::to_owned),
            };
            deliver(Err(AgentError {
                code: "io".into(),
                message: "disk full".into(),
            }));
            deliver(Ok(AgentResponse::Hooks(report(HooksState::Missing, None))));
            deliver(Ok(AgentResponse::Hooks(HooksReport {
                agent: AgentKind::Claude,
                ..report(HooksState::Installed, None)
            })));
            deliver(Ok(AgentResponse::Hooks(report(
                HooksState::Installed,
                Some("enable hooks by hand"),
            ))));
            let connection = this.connection(1).unwrap();
            assert_eq!(
                connection.hooks_error, None,
                "a report clears the last error"
            );
            assert_eq!(connection.hooks.len(), 2, "one row per agent");
            let codex = connection
                .hooks
                .iter()
                .find(|report| report.agent == AgentKind::Codex)
                .unwrap();
            assert_eq!(codex.state, HooksState::Installed);
            assert_eq!(codex.warning.as_deref(), Some("enable hooks by hand"));
        });
    });
}

#[test]
fn the_daemon_page_keeps_half_typed_addresses_local_and_invites_follow_the_listener() {
    use condr_core::protocol::{ServerAdminResponse, ServerMessage};
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        gpui_kit::init(cx);
        super::super::super::startup::bind_keys(cx);
    });
    let (view, window, _server) = connected_condr(&mut cx);
    let main_window = window.update(|window, _| window.window_handle());
    let settings_button = window.debug_bounds("open-settings").unwrap();
    window.simulate_click(settings_button.center(), Modifiers::default());
    window.run_until_parked();
    let settings_handle = window
        .windows()
        .into_iter()
        .find(|handle| *handle != main_window)
        .unwrap();
    let settings = VisualTestContext::from_window(settings_handle, window).into_mut();
    settings.update(|window, cx| _ = window.draw(cx));
    let settings_view = settings.read(|app| {
        view.read(app)
            .settings_view
            .as_ref()
            .and_then(|view| view.upgrade())
            .unwrap()
    });

    // A partial address stays in the field and never reaches the Server.
    let before = settings.read(|app| view.read(app).connection(1).unwrap().listen.clone());
    settings.update(|_, cx| set_server_listen(&settings_view, "127.0.0.1:".into(), cx));
    assert_eq!(
        settings.read(|app| server_listen(&settings_view, app)),
        "127.0.0.1:"
    );
    settings.update(|_, cx| set_server_listen(&settings_view, "not an address".into(), cx));
    settings.run_until_parked();
    assert_eq!(
        settings.read(|app| view.read(app).connection(1).unwrap().listen.clone()),
        before,
        "an incomplete address is not saved"
    );

    settings.update(|_, cx| {
        view.update(cx, |this, cx| {
            let generation = this.connection(1).unwrap().connect_generation;
            this.handle_incoming(
                1,
                generation,
                Incoming::Message(ServerMessage::ServerAdmin(ServerAdminResponse::Invite {
                    tcp: Some("tcp://key.secret@<host>:2637".into()),
                    p2p: Some("p2p://key.secret".into()),
                    expires_in_secs: 600,
                })),
                cx,
            );
            let invite = this.connection(1).unwrap().invite.clone().unwrap();
            assert_eq!(invite.tcp.as_deref(), Some("tcp://key.secret@<host>:2637"));
            assert_eq!(invite.p2p.as_deref(), Some("p2p://key.secret"));
            assert_eq!(invite.expires_in_secs, 600);
        });
    });
    // The invite lives on the Server tab's Paired devices page, its third page.
    settings.update(|_, cx| {
        select_settings_server_page(&settings_view, 2, cx);
        select_settings_tab(&settings_view, SettingsTab::Server, cx);
    });
    settings.update(|window, cx| _ = window.draw(cx));
    assert!(
        settings.debug_bounds("server-invite-tcp").is_some()
            && settings.debug_bounds("server-invite-p2p").is_some(),
        "one link per enabled transport"
    );

    settings.update(|_, cx| {
        view.update(cx, |this, cx| {
            let generation = this.connection(1).unwrap().connect_generation;
            this.handle_incoming(
                1,
                generation,
                Incoming::Message(ServerMessage::ServerAdmin(ServerAdminResponse::P2pSaved {
                    enabled: true,
                })),
                cx,
            );
            let connection = this.connection(1).unwrap();
            assert!(connection.p2p, "the p2p setting follows the Server's reply");
            assert_eq!(connection.invite, None, "a p2p change voids the old invite");
            this.handle_incoming(
                1,
                generation,
                Incoming::Message(ServerMessage::ServerAdmin(ServerAdminResponse::Invite {
                    tcp: Some("tcp://key.secret@<host>:2637".into()),
                    p2p: None,
                    expires_in_secs: 600,
                })),
                cx,
            );
            this.handle_incoming(
                1,
                generation,
                Incoming::Message(ServerMessage::ServerAdmin(
                    ServerAdminResponse::ListenSaved {
                        listen: Some("127.0.0.1:2638".into()),
                    },
                )),
                cx,
            );
            let connection = this.connection(1).unwrap();
            assert_eq!(connection.listen.as_deref(), Some("127.0.0.1:2638"));
            assert_eq!(connection.invite, None, "a new port voids the old invite");
        });
    });
    settings.update(|window, cx| _ = window.draw(cx));
    assert!(
        settings.debug_bounds("server-invite-tcp").is_none()
            && settings.debug_bounds("server-invite-p2p").is_none(),
        "no invite is shown after the listener changed"
    );
}

#[test]
fn the_settings_window_draws_its_confirm_dialogs() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(|cx| {
        gpui_kit::init(cx);
        super::super::super::startup::bind_keys(cx);
    });
    let (view, window, _server) = connected_condr(&mut cx);
    let main_window = window.update(|window, _| window.window_handle());
    let settings_button = window.debug_bounds("open-settings").unwrap();
    window.simulate_click(settings_button.center(), Modifiers::default());
    window.run_until_parked();
    let settings_handle = window
        .windows()
        .into_iter()
        .find(|handle| *handle != main_window)
        .unwrap();
    let settings = VisualTestContext::from_window(settings_handle, window).into_mut();
    let settings_view = settings.read(|app| {
        view.read(app)
            .settings_view
            .as_ref()
            .and_then(|view| view.upgrade())
            .unwrap()
    });
    settings.update(|_, cx| select_settings_tab(&settings_view, SettingsTab::Server, cx));
    settings.update(|window, cx| _ = window.draw(cx));

    // The same confirm the Restart and Revoke buttons open; Root does not draw dialogs
    // on its own, so the window must mount the layer or the click looks ignored.
    settings.update(|window, cx| {
        window.open_alert_dialog(cx, |alert, _, _| alert.confirm().title("Restart Server?"));
    });
    settings.run_until_parked();
    settings.update(|window, cx| _ = window.draw(cx));
    assert!(settings.update(|window, cx| window.has_active_dialog(cx)));
    // Escape reaches the dialog only if it is drawn and focused; otherwise the window's
    // own Escape handler closes Settings instead. (Confirming for real would restart the
    // in-process test Server through the test binary.)
    settings.simulate_keystrokes("escape");
    settings.run_until_parked();
    assert_eq!(
        window.windows().len(),
        2,
        "Escape closed the dialog, not Settings"
    );
    assert!(!settings.update(|window, cx| window.has_active_dialog(cx)));

    // Once asked, the page says so until the Server is back.
    settings.update(|_, cx| {
        view.update(cx, |this, cx| this.restart_server(1, cx));
    });
    assert!(settings.read(|app| {
        view.read(app)
            .connection(1)
            .unwrap()
            .reconnect_deadline
            .is_some()
    }));
}

#[test]
fn only_the_check_button_reports_a_failed_update_check() {
    use crate::app::updates::{CHECK_INTERVAL, FIRST_CHECK_DELAY, UpdateChannel, UpdateState};
    use gpui_kit::http_client::{FakeHttpClient, Response};

    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr(&mut cx);
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.set_update_channel(UpdateChannel::Stable, cx)
        })
    });
    let state =
        |window: &mut VisualTestContext| window.read(|app| view.read(app).update_state.clone());

    // GPUI's test client answers 404: the automatic check only logs it.
    window.executor().advance_clock(FIRST_CHECK_DELAY);
    window.run_until_parked();
    assert_eq!(state(window), UpdateState::Unknown);
    assert!(window.read(|app| view.read(app).last_error.is_none()));

    window.update(|_, cx| view.update(cx, |this, cx| this.check_for_updates_now(cx)));
    assert!(window.read(|app| view.read(app).checking_updates));
    window.run_until_parked();
    assert!(!window.read(|app| view.read(app).checking_updates));
    assert!(
        window
            .read(|app| view.read(app).last_error.clone())
            .is_some_and(|error| error.starts_with("Failed to check for updates")),
        "a failed Check must be reported"
    );

    window.update(|_, cx| {
        cx.set_http_client(FakeHttpClient::create(|_| async {
            Ok(Response::builder()
                .status(200)
                .body(r#"{"tag_name":"v99.0.0"}"#.into())
                .unwrap())
        }))
    });
    let found = |window: &mut VisualTestContext| matches!(state(window), UpdateState::Available(update) if update.name.as_ref() == "Condr 99.0.0");
    // The next automatic check comes five hours after the last, whatever Check did.
    window.executor().advance_clock(CHECK_INTERVAL);
    window.run_until_parked();
    assert!(found(window), "the automatic checks must repeat");

    // Turning automatic checks off forgets what they found; Check still works.
    window.update(|_, cx| view.update(cx, |this, cx| this.set_auto_check_updates(false, cx)));
    assert_eq!(state(window), UpdateState::Unknown);
    window.update(|_, cx| view.update(cx, |this, cx| this.check_for_updates_now(cx)));
    window.run_until_parked();
    assert!(found(window));
}
