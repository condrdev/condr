use super::*;

/// The shell a Server currently stores, as its Bootstrap or last event reported it.
pub(super) fn connection_shell(
    owner: &WeakEntity<Condr>,
    key: ConnectionKey,
    cx: &App,
) -> SharedString {
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
pub(in crate::app) fn select_settings_server(
    settings: &Entity<SettingsWindow>,
    key: ConnectionKey,
    cx: &mut App,
) {
    settings.update(cx, |this, cx| this.select_server(key, cx));
}

/// What clicking a tab in the tab bar does; tests call it without the widget.
#[cfg(all(test, feature = "test-support"))]
pub(in crate::app) fn select_settings_tab(
    settings: &Entity<SettingsWindow>,
    tab: SettingsTab,
    cx: &mut App,
) {
    settings.update(cx, |this, cx| {
        this.tab = tab;
        cx.notify();
    });
}

/// What the Shell field shows.
pub(in crate::app) fn server_shell(settings: &Entity<SettingsWindow>, cx: &App) -> SharedString {
    settings.read(cx).shell_draft.clone()
}

/// What the Shell field does on every change.
pub(in crate::app) fn select_server_shell(
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

/// Preferences a Server owns, edited for one connection at a time. Only the shell so
/// far; the Server picker sits in the tab bar.
pub(super) fn server_page(settings: &Entity<SettingsWindow>) -> SettingPage {
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
        .group(server_network_group(settings))
        .group(server_clients_group(settings))
}

fn server_network_group(settings: &Entity<SettingsWindow>) -> SettingGroup {
    let value = settings.clone();
    let set = settings.clone();
    let restart = settings.clone();
    SettingGroup::new()
        .title("TCP listener")
        .item(
            SettingItem::new(
                "Listen address",
                SettingField::input(
                    move |cx| server_listen(&value, cx),
                    move |address: SharedString, cx| set_server_listen(&set, address, cx),
                )
                .default_value(""),
            )
            .description("Empty disables TCP. Restart the Server to apply the saved address."),
        )
        .item(SettingItem::render(move |_, _, cx| {
            let settings = restart.clone();
            let (allowed, error) = settings
                .read(cx)
                .owner
                .upgrade()
                .and_then(|owner| {
                    let owner = owner.read(cx);
                    owner
                        .connections
                        .iter()
                        .find(|c| c.key == settings.read(cx).selected_server)
                        .map(|c| {
                            (
                                matches!(c.endpoint, Endpoint::Local(_) | Endpoint::Ssh(_)),
                                c.error.clone(),
                            )
                        })
                })
                .unwrap_or((false, None));
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(error.unwrap_or_else(|| {
                            if allowed {
                                "Configuration follows the CLI."
                            } else {
                                "Read-only for TCP connections."
                            }
                            .into()
                        })),
                )
                .child(
                    Button::new("server-restart")
                        .label("Restart Server")
                        .small()
                        .outline()
                        .disabled(!allowed)
                        .on_click(move |_, window, cx| {
                            let (owner, key) = {
                                let this = settings.read(cx);
                                (this.owner.clone(), this.selected_server)
                            };
                            window.defer(cx, move |window, cx| {
                                window.open_alert_dialog(cx, move |alert, _, _| {
                                    let owner = owner.clone();
                                    alert
                                        .confirm()
                                        .title("Restart Server?")
                                        .description("All panes and agent processes will stop.")
                                        .on_ok(move |_, _, cx| {
                                            let _ = owner.update(cx, |owner, _| {
                                                owner.server_admin(key, ServerAdminCommand::Restart)
                                            });
                                            true
                                        })
                                });
                            });
                        }),
                )
        }))
}

fn server_clients_group(settings: &Entity<SettingsWindow>) -> SettingGroup {
    let invite_settings = settings.clone();
    SettingGroup::new().title("Clients").item(
        SettingItem::render(move |_, _, cx| {
            let settings = invite_settings.clone();
            let (allowed, invite, clients, connected) = settings
                .read(cx)
                .owner
                .upgrade()
                .and_then(|owner| {
                    let owner = owner.read(cx);
                    owner
                        .connections
                        .iter()
                        .find(|c| c.key == settings.read(cx).selected_server)
                        .map(|c| {
                            (
                                matches!(c.endpoint, Endpoint::Local(_) | Endpoint::Ssh(_)),
                                c.invite.clone(),
                                c.clients.clone(),
                                c.connected_devices.clone(),
                            )
                        })
                })
                .unwrap_or_default();
            let mut row = v_flex().gap_2();
            if allowed {
                let settings = settings.clone();
                row = row.child(
                    Button::new("server-invite")
                        .label("Generate invite")
                        .small()
                        .outline()
                        .on_click(move |_, _, cx| {
                            settings.update(cx, |this, cx| {
                                let key = this.selected_server;
                                let _ = this.owner.update(cx, |owner, _| {
                                    owner.server_admin(key, ServerAdminCommand::Invite)
                                });
                            });
                        }),
                );
            }
            if let Some(invite) = invite {
                row = row.child(div().text_sm().child(invite));
            }
            if clients.is_empty() {
                row = row.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("No paired clients."),
                );
            } else {
                for client in clients {
                    let connected = connected.contains(&client.fingerprint);
                    let key = client.fingerprint.clone();
                    let settings = settings.clone();
                    let row_view = h_flex()
                        .gap_2()
                        .items_center()
                        .child(div().flex_1().child(format!(
                            "{}  {}  {}",
                            client.name,
                            client.last_seen,
                            if connected { "connected" } else { "" }
                        )))
                        .when(allowed, |this| {
                            this.child(
                                Button::new(format!("revoke-{key}"))
                                    .label("Revoke")
                                    .small()
                                    .outline()
                                    .on_click(move |_, _, cx| {
                                        settings.update(cx, |this, cx| {
                                            let selected = this.selected_server;
                                            let _ = this.owner.update(cx, |owner, _| {
                                                owner.server_admin(
                                                    selected,
                                                    ServerAdminCommand::Revoke { key: key.clone() },
                                                )
                                            });
                                        });
                                    }),
                            )
                        });
                    row = row.child(row_view);
                }
            }
            row.into_any_element()
        })
        .keywords(["clients", "invite", "revoke"]),
    )
}

fn server_listen(settings: &Entity<SettingsWindow>, cx: &App) -> SharedString {
    settings
        .read(cx)
        .owner
        .upgrade()
        .and_then(|owner| {
            owner
                .read(cx)
                .connections
                .iter()
                .find(|c| c.key == settings.read(cx).selected_server)
                .and_then(|c| c.listen.clone())
        })
        .unwrap_or_default()
        .into()
}

fn set_server_listen(settings: &Entity<SettingsWindow>, address: SharedString, cx: &mut App) {
    let (key, allowed) = {
        let this = settings.read(cx);
        let key = this.selected_server;
        let allowed = this.owner.upgrade().is_some_and(|owner| {
            owner
                .read(cx)
                .connections
                .iter()
                .find(|connection| connection.key == key)
                .is_some_and(|connection| {
                    matches!(connection.endpoint, Endpoint::Local(_) | Endpoint::Ssh(_))
                })
        });
        (key, allowed)
    };
    if !allowed {
        return;
    }
    let command = ServerAdminCommand::SaveListen {
        address: (!address.trim().is_empty()).then(|| address.to_string()),
    };
    let owner = settings.read(cx).owner.clone();
    let _ = owner.update(cx, |owner, _| owner.server_admin(key, command));
}

/// What one hooks action does: asks the selected Server and lets the reply repaint.
pub(in crate::app) fn run_agent_hooks(
    settings: &Entity<SettingsWindow>,
    agent: AgentKind,
    action: HooksAction,
    cx: &mut App,
) {
    settings.update(cx, |this, cx| {
        let key = this.selected_server;
        let _ = this
            .owner
            .update(cx, |owner, _| owner.set_agent_hooks(key, agent, action));
        cx.notify();
    });
}

/// The status hooks of every supported agent on the selected Server's machine, with the
/// actions their state allows; the Server writes the agent's own configuration (ADR 0014).
/// Built on every render, so the rows follow the latest reports.
pub(super) fn agents_page(
    settings: &Entity<SettingsWindow>,
    (reports, error): (Vec<HooksReport>, Option<String>),
) -> SettingPage {
    let mut group = SettingGroup::new().title("Status hooks");
    if let Some(error) = error {
        group = group.item(
            SettingItem::render(move |_, _, cx| {
                div()
                    .text_color(cx.theme().danger)
                    .child(format!("Hooks request failed: {error}"))
            })
            .keywords(["hooks", "error"]),
        );
    }
    for agent in AgentKind::ALL {
        let report = reports.iter().find(|report| report.agent == agent).cloned();
        let description = match &report {
            Some(report) => report
                .warning
                .clone()
                .unwrap_or_else(|| report.path.display().to_string()),
            None => "Waiting for the Server to report.".to_owned(),
        };
        let settings = settings.clone();
        // A custom row rather than `SettingItem::new`: the title carries the agent's
        // mark, which the standard title slot cannot.
        group = group.item(
            SettingItem::render(move |_, _, cx| {
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_4()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        super::sidebar::agent_mark(agent, cx.theme().foreground)
                                            .small(),
                                    )
                                    .child(agent.label()),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(description.clone()),
                            ),
                    )
                    .child(agent_hooks_field(&settings, agent, report.as_ref(), cx))
            })
            .keywords([agent.label(), "hooks"]),
        );
    }
    SettingPage::new("Agents").icon(IconName::Bot).group(group)
}

/// One row's field: the state as a word, then the one or two actions that change it.
/// Installed hooks only offer Uninstall; an outdated set offers Update beside it. A
/// missing report disables everything rather than guessing.
pub(super) fn agent_hooks_field(
    settings: &Entity<SettingsWindow>,
    agent: AgentKind,
    report: Option<&HooksReport>,
    cx: &App,
) -> AnyElement {
    let state = report.map(|report| report.state);
    let (state_label, color) = match state {
        Some(HooksState::Installed) => ("Installed", cx.theme().success),
        Some(HooksState::Outdated) => ("Outdated", cx.theme().warning),
        Some(HooksState::Missing) => ("Not installed", cx.theme().muted_foreground),
        None => ("Checking…", cx.theme().muted_foreground),
    };
    let install_label = if state == Some(HooksState::Outdated) {
        "Update"
    } else {
        "Install"
    };
    let action = |id: &'static str, label: &'static str, action: HooksAction| {
        let settings = settings.clone();
        Button::new(format!("agent-hooks-{}-{id}", agent.id()))
            .label(label)
            .small()
            .outline()
            .disabled(report.is_none())
            .on_click(move |_, _, cx| run_agent_hooks(&settings, agent, action, cx))
    };
    h_flex()
        .flex_none()
        .gap_2()
        .items_center()
        .child(
            div()
                .debug_selector(move || format!("agent-hooks-{}-state", agent.id()))
                .whitespace_nowrap()
                .text_color(color)
                .child(state_label),
        )
        .when(state != Some(HooksState::Installed), |this| {
            this.child(action("install", install_label, HooksAction::Install))
        })
        .when(
            matches!(state, Some(HooksState::Installed | HooksState::Outdated)),
            |this| this.child(action("uninstall", "Uninstall", HooksAction::Uninstall)),
        )
        .into_any_element()
}

impl Condr {
    /// Asks a Server to store a new shell preference. The stored value comes back as a
    /// `ServerSettingsChanged` event; nothing is assumed locally.
    pub(in crate::app) fn set_server_shell(
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
    pub(in crate::app) fn flush_server_shell(&mut self) {
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

    /// Asks a Server for the state of every agent's hooks on its machine. The replies
    /// arrive as `AgentResult`s and land on the connection, one row per agent.
    pub(in crate::app) fn request_agent_hooks(&mut self, key: ConnectionKey) {
        if let Some(connection) = self.connections.iter_mut().find(|c| c.key == key) {
            for agent in AgentKind::ALL {
                connection.send_agent_hooks(agent, HooksAction::Status);
            }
        }
    }

    /// Installs or removes one agent's hooks on a Server's machine; the reply replaces
    /// that agent's row.
    pub(in crate::app) fn set_agent_hooks(
        &mut self,
        key: ConnectionKey,
        agent: AgentKind,
        action: HooksAction,
    ) {
        if let Some(connection) = self.connections.iter_mut().find(|c| c.key == key) {
            connection.send_agent_hooks(agent, action);
        }
    }
}
