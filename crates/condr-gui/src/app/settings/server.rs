use super::*;
use gpui_kit::component::clipboard::Clipboard;
use gpui_kit::component::tag::Tag;

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

/// What the Shell field shows; tests read it without the widget.
#[cfg(all(test, feature = "test-support"))]
pub(in crate::app) fn server_shell(settings: &Entity<SettingsWindow>, cx: &App) -> SharedString {
    settings.read(cx).shell.draft.clone()
}

/// What typing a shell and leaving the field does; tests call it without the widget.
#[cfg(all(test, feature = "test-support"))]
pub(in crate::app) fn select_server_shell(
    settings: &Entity<SettingsWindow>,
    shell: SharedString,
    cx: &mut App,
) {
    settings.update(cx, |this, cx| {
        this.shell.draft = shell;
        this.commit(TextFieldId::Shell, cx);
    });
}

const DEFAULT_LISTEN: &str = "127.0.0.1:2637";

/// Preferences a Server owns, edited for one connection at a time; the Server picker sits
/// in the tab bar. Each page mirrors one `condr server` concern.
pub(super) fn server_terminal_page(settings: &Entity<SettingsWindow>) -> SettingPage {
    SettingPage::new("Terminal")
        .icon(IconName::SquareTerminal)
        .default_open(true)
        .group(
            SettingGroup::new().item(
                SettingItem::new(
                    "Shell",
                    text_field_row(settings, TextFieldId::Shell, "".into()),
                )
                .description("Empty uses the system default."),
            ),
        )
}

/// `this` is the window being rendered, so it must not go through `settings.read(cx)`.
pub(super) fn server_network_page(
    this: &SettingsWindow,
    settings: &Entity<SettingsWindow>,
    cx: &App,
) -> SettingPage {
    SettingPage::new("Daemon")
        .icon(IconName::Cpu)
        .group(server_status_group(this, cx))
        .group(server_network_group(this, settings, cx))
}

pub(super) fn server_clients_page(settings: &Entity<SettingsWindow>) -> SettingPage {
    SettingPage::new("Paired devices")
        .icon(IconName::Network)
        .default_open(true)
        .group(server_clients_group(settings))
}

/// Whether this Client may change the selected Server: only a local or SSH connection
/// can, and the Server refuses everything else (ADR 0015).
fn server_admin_allowed(settings: &Entity<SettingsWindow>, cx: &App) -> bool {
    admin_allowed(settings.read(cx), cx)
}

fn admin_allowed(this: &SettingsWindow, cx: &App) -> bool {
    this.selected_connection(cx, |c| {
        c.status == ConnectionStatus::Connected
            && matches!(c.endpoint, Endpoint::Local(_) | Endpoint::Ssh(_))
    })
    .unwrap_or(false)
}

fn selected_connection<T>(
    settings: &Entity<SettingsWindow>,
    cx: &App,
    read: impl FnOnce(&ServerConnection) -> T,
) -> Option<T> {
    settings.read(cx).selected_connection(cx, read)
}

impl SettingsWindow {
    fn selected_connection<T>(
        &self,
        cx: &App,
        read: impl FnOnce(&ServerConnection) -> T,
    ) -> Option<T> {
        let owner = self.owner.upgrade()?;
        owner
            .read(cx)
            .connections
            .iter()
            .find(|c| c.key == self.selected_server)
            .map(read)
    }

    /// Saves the Listen address draft if it is a complete `host:port`; anything else is
    /// refused and stays in the field. With the listener off nothing goes out: the
    /// address is used when it is switched on. See `SettingsWindow::commit`.
    pub(in crate::app) fn commit_listen(&mut self, cx: &mut Context<Self>) -> Option<bool> {
        let Ok(address) = self.listen.draft.trim().parse::<std::net::SocketAddr>() else {
            self.listen_refused = true;
            return None;
        };
        self.listen_refused = false;
        let listening = self
            .selected_connection(cx, |c| c.listen.is_some())
            .unwrap_or(false);
        if listening {
            self.send_listen(Some(address.to_string()), cx);
        }
        Some(listening)
    }

    fn send_listen(&mut self, address: Option<String>, cx: &mut Context<Self>) {
        if !admin_allowed(self, cx) {
            return;
        }
        let key = self.selected_server;
        let command = ServerAdminCommand::SaveListen { address };
        let _ = self
            .owner
            .update(cx, |owner, _| owner.server_admin(key, command));
    }
}

/// Which Server this page describes and what it last reported about itself: the
/// connection (a Server can run while this GUI is disconnected, so it never claims
/// "running"), version, uptime, Session counts and the `warn`/`error` records it kept.
/// Health is refreshed every `STATUS_REFRESH` while Settings is open.
fn server_status_group(this: &SettingsWindow, cx: &App) -> SettingGroup {
    let (status, kind, address) = this
        .selected_connection(cx, |c| {
            let (kind, address) = match &c.endpoint {
                Endpoint::Local(_) => ("Local", None),
                Endpoint::Ssh(ssh) => ("SSH", Some(ssh.destination().to_owned())),
                Endpoint::Tcp(tcp) => ("TCP", Some(tcp.authority())),
                Endpoint::P2p(p2p) => ("Peer-to-peer", Some(p2p.device.to_hex())),
            };
            (c.status, kind, address)
        })
        .unwrap_or((ConnectionStatus::Disconnected, "Local", None));
    let state = match status {
        ConnectionStatus::Connected => "Connected",
        ConnectionStatus::Connecting => "Connecting",
        ConnectionStatus::Disconnected => "Disconnected",
    };
    let status_row = SettingItem::new(
        "Status",
        SettingField::render(move |_, _, _| {
            match status {
                ConnectionStatus::Connected => Tag::success(),
                ConnectionStatus::Connecting => Tag::warning(),
                ConnectionStatus::Disconnected => Tag::secondary(),
            }
            .outline()
            .small()
            .child(state)
        }),
    )
    .keywords(["status", "connected"]);
    let connection = SettingItem::new(
        "Connection",
        SettingField::render(move |_, _, _| div().text_sm().child(kind)),
    )
    .keywords(["connection", "local", "ssh", "tcp"]);
    let connection = match address {
        Some(address) => connection.description(SharedString::from(address)),
        None => connection,
    };
    let group = SettingGroup::new()
        .title("Status")
        .item(status_row)
        .item(connection);

    let health = this.selected_connection(cx, |c| c.health.clone()).flatten();
    let Some(health) = health else {
        return group.item(SettingItem::render(|_, _, cx| {
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child("Waiting for the Server's status…")
        }));
    };
    let value =
        |text: String| SettingField::render(move |_, _, _| div().text_sm().child(text.clone()));
    let this_version = env!("CARGO_PKG_VERSION");
    let mismatch = health.version != this_version;
    let version = health.version.clone();
    let version = SettingItem::new(
        "Version",
        SettingField::render(move |_, _, cx| {
            div()
                .text_sm()
                .when(mismatch, |this| this.text_color(cx.theme().warning))
                .child(version.clone())
        }),
    )
    .keywords(["version"]);
    let version = if mismatch {
        version.description(SharedString::from(format!(
            "This window is {this_version}."
        )))
    } else {
        version
    };
    let session = [
        count(health.workspaces, "workspace"),
        count(health.tabs, "tab"),
        count(health.panes, "pane"),
        count(health.agents, "agent"),
        count(health.clients, "client"),
    ]
    .join(" · ");
    let errors = health.recent_errors;
    let mut group = group
        .item(version)
        .item(
            SettingItem::new("Uptime", value(uptime_text(health.uptime_secs))).keywords(["uptime"]),
        )
        .item(SettingItem::new("Session", value(session)).keywords(["workspaces", "panes"]));
    if !errors.is_empty() {
        group = group.item(
            SettingItem::new(
                "Recent errors",
                SettingField::render(move |_, _, cx| {
                    let muted = cx.theme().muted_foreground;
                    v_flex()
                        .gap_1()
                        .text_sm()
                        .children(errors.iter().rev().map(|record| {
                            let color = if record.level == "ERROR" {
                                cx.theme().danger
                            } else {
                                cx.theme().warning
                            };
                            h_flex()
                                .gap_2()
                                .items_start()
                                .child(div().text_color(color).child(record.level.clone()))
                                .child(div().text_color(muted).child(relative_age(record.at)))
                                .child(record.message.clone())
                        }))
                }),
            )
            .layout(Axis::Vertical)
            .keywords(["errors", "health"]),
        );
    }
    group
}

fn count(n: u32, noun: &str) -> String {
    format!("{n} {noun}{}", if n == 1 { "" } else { "s" })
}

fn server_restart_row(settings: &Entity<SettingsWindow>) -> SettingItem {
    let restart = settings.clone();
    SettingItem::new(
        "Restart Condr",
        SettingField::render(move |_, _, cx| {
            let settings = restart.clone();
            let allowed = server_admin_allowed(&settings, cx);
            let restarting = selected_connection(&settings, cx, |c| c.reconnect_deadline.is_some())
                .unwrap_or_default();
            h_flex()
                .gap_3()
                .items_center()
                .when(restarting, |row| {
                    row.child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child("Waiting for Condr to come back…"),
                    )
                })
                .child(
                    Button::new("server-restart")
                        .label(if restarting {
                            "Restarting…"
                        } else {
                            "Restart Condr"
                        })
                        .small()
                        .outline()
                        .disabled(!allowed || restarting)
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
                                        .title("Restart Condr on this device?")
                                        .description(
                                            "Every pane and agent on this device stops; Condr \
                                         windows reconnect on their own.",
                                        )
                                        .button_props(
                                            DialogButtonProps::default()
                                                .ok_text("Restart")
                                                .ok_variant(ButtonVariant::Danger),
                                        )
                                        .on_ok(move |_, _, cx| {
                                            let _ = owner.update(cx, |owner, cx| {
                                                owner.restart_server(key, cx)
                                            });
                                            true
                                        })
                                });
                            });
                        }),
                )
        }),
    )
    .description("Applies the changes above. Stops every pane and agent on this device.")
    .keywords(["restart", "apply"])
}

fn server_network_group(
    this: &SettingsWindow,
    settings: &Entity<SettingsWindow>,
    cx: &App,
) -> SettingGroup {
    let over_tcp = this
        .selected_connection(cx, |c| matches!(c.endpoint, Endpoint::Tcp(_)))
        .unwrap_or(false);
    let group = SettingGroup::new().title("Network");
    // The Server refuses admin commands over TCP (ADR 0015); say so only when it applies.
    let group = if over_tcp {
        group.description("Read-only over TCP. Connect locally or over SSH to change these.")
    } else {
        group
    };
    group
        .item(SettingItem::new(
            "TCP listener",
            SettingField::switch(
                {
                    let settings = settings.clone();
                    move |cx| server_listen_enabled(&settings, cx)
                },
                {
                    let settings = settings.clone();
                    move |enabled, cx| set_server_listen_enabled(&settings, enabled, cx)
                },
            )
            .default_value(false),
        ))
        .item(
            SettingItem::new(
                "Listen address",
                text_field_row(settings, TextFieldId::Listen, DEFAULT_LISTEN.into()),
            )
            .description("host:port"),
        )
        .item(server_restart_row(settings))
}

fn server_clients_group(settings: &Entity<SettingsWindow>) -> SettingGroup {
    let invite_settings = settings.clone();
    let list_settings = settings.clone();
    SettingGroup::new()
        .item(
            SettingItem::render(move |_, _, cx| {
                let settings = invite_settings.clone();
                let allowed = server_admin_allowed(&settings, cx);
                let (listening, invite) = settings
                    .read(cx)
                    .owner
                    .upgrade()
                    .and_then(|owner| {
                        let this = settings.read(cx);
                        owner
                            .read(cx)
                            .connections
                            .iter()
                            .find(|c| c.key == this.selected_server)
                            .map(|c| (c.listen.is_some(), c.invite.clone()))
                    })
                    .unwrap_or_default();
                let hint = match (allowed, listening) {
                    (false, _) => "Only a local or SSH connection can invite devices.",
                    (true, false) => "Turn on the TCP listener first: devices pair over TCP.",
                    (true, true) => {
                        "A one-time address for Connect Remote Device on the new device."
                    }
                };
                let mut column = v_flex().gap_2().child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(
                            Button::new("server-invite")
                                .label(if invite.is_some() {
                                    "New invite"
                                } else {
                                    "Generate invite"
                                })
                                .small()
                                .outline()
                                .disabled(!(allowed && listening))
                                .on_click({
                                    let settings = settings.clone();
                                    move |_, _, cx| {
                                        settings.update(cx, |this, cx| {
                                            let key = this.selected_server;
                                            let _ = this.owner.update(cx, |owner, _| {
                                                owner.server_admin(key, ServerAdminCommand::Invite)
                                            });
                                        });
                                    }
                                }),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(hint),
                        ),
                );
                if let Some((address, expires_in_secs)) = invite {
                    column = column.child(
                        v_flex()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .debug_selector(|| "server-invite-address".into())
                                            .min_w_0()
                                            .truncate()
                                            .text_sm()
                                            .font_family("monospace")
                                            .child(address.clone()),
                                    )
                                    .child(
                                        Clipboard::new("server-invite-copy")
                                            .value(address)
                                            .tooltip("Copy invite"),
                                    ),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "Copied. Valid for {} minutes; replace <host> with an \
                                         address the new device can reach.",
                                        expires_in_secs.div_ceil(60)
                                    )),
                            ),
                    );
                }
                column.into_any_element()
            })
            .keywords(["invite", "pair"]),
        )
        .item(
            SettingItem::render(move |_, _, cx| {
                let settings = list_settings.clone();
                let allowed = server_admin_allowed(&settings, cx);
                let (clients, connected) = settings
                    .read(cx)
                    .owner
                    .upgrade()
                    .and_then(|owner| {
                        let this = settings.read(cx);
                        owner
                            .read(cx)
                            .connections
                            .iter()
                            .find(|c| c.key == this.selected_server)
                            .map(|c| (c.clients.clone(), c.connected_devices.clone()))
                    })
                    .unwrap_or_default();
                if clients.is_empty() {
                    return div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child("No paired devices.")
                        .into_any_element();
                }
                // The same columns as `condr server clients`.
                let header = |text: &'static str| {
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(text)
                };
                let mut table = v_flex().gap_2().child(
                    h_flex()
                        .gap_4()
                        .items_center()
                        .child(div().w(rems(10.)).child(header("NAME")))
                        .child(div().w(rems(8.)).child(header("LAST SEEN")))
                        .child(div().flex_1().child(header("FINGERPRINT"))),
                );
                for client in clients {
                    let seen = if connected.contains(&client.fingerprint) {
                        "connected".to_owned()
                    } else {
                        relative_age(client.last_seen)
                    };
                    let key = client.fingerprint.clone();
                    let name = client.name.clone();
                    let settings = settings.clone();
                    table = table.child(
                        h_flex()
                            .gap_4()
                            .items_center()
                            .text_sm()
                            .child(div().w(rems(10.)).truncate().child(client.name))
                            .child(div().w(rems(8.)).whitespace_nowrap().child(seen))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .font_family("monospace")
                                    .child(client.fingerprint),
                            )
                            .when(allowed, |this| {
                                this.child(
                                    Button::new(format!("revoke-{key}"))
                                        .label("Revoke")
                                        .small()
                                        .outline()
                                        .on_click(move |_, window, cx| {
                                            let (owner, selected) = {
                                                let this = settings.read(cx);
                                                (this.owner.clone(), this.selected_server)
                                            };
                                            let key = key.clone();
                                            let name = name.clone();
                                            window.defer(cx, move |window, cx| {
                                                window.open_alert_dialog(cx, move |alert, _, _| {
                                                    let owner = owner.clone();
                                                    let key = key.clone();
                                                    alert
                                                        .confirm()
                                                        .title(format!(
                                                            "Revoke \u{201c}{name}\u{201d}?"
                                                        ))
                                                        .description(
                                                            "Its connections close now, and it \
                                                             needs a new invite to pair again.",
                                                        )
                                                        .button_props(
                                                            DialogButtonProps::default()
                                                                .ok_text("Revoke")
                                                                .ok_variant(ButtonVariant::Danger),
                                                        )
                                                        .on_ok(move |_, _, cx| {
                                                            let _ = owner.update(cx, |owner, _| {
                                                                owner.server_admin(
                                                                    selected,
                                                                    ServerAdminCommand::Revoke {
                                                                        key: key.clone(),
                                                                    },
                                                                )
                                                            });
                                                            true
                                                        })
                                                });
                                            });
                                        }),
                                )
                            }),
                    );
                }
                table.into_any_element()
            })
            .keywords(["clients", "devices", "revoke"]),
        )
}

/// The listen address a Server stores, or the default when it has none.
pub(super) fn connection_listen(
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
                .find(|c| c.key == key)
                .and_then(|c| c.listen.clone())
        })
        .unwrap_or_else(|| DEFAULT_LISTEN.to_owned())
        .into()
}

/// What the Listen address field shows: the draft, so typing is not rewritten under the
/// user while the Server has only the last complete address.
pub(in crate::app) fn server_listen(settings: &Entity<SettingsWindow>, cx: &App) -> SharedString {
    settings.read(cx).listen.draft.clone()
}

fn server_listen_enabled(settings: &Entity<SettingsWindow>, cx: &App) -> bool {
    selected_connection(settings, cx, |c| c.listen.is_some()).unwrap_or(false)
}

fn set_server_listen_enabled(settings: &Entity<SettingsWindow>, enabled: bool, cx: &mut App) {
    let address = enabled
        .then(|| {
            server_listen(settings, cx)
                .parse::<std::net::SocketAddr>()
                .ok()
        })
        .flatten();
    if enabled && address.is_none() {
        return;
    }
    set_server_listen_value(settings, address.map(|address| address.to_string()), cx);
}

/// What leaving the Listen address field does; tests call it without the widget. See
/// `SettingsWindow::commit_listen`.
#[cfg(all(test, feature = "test-support"))]
pub(in crate::app) fn set_server_listen(
    settings: &Entity<SettingsWindow>,
    address: SharedString,
    cx: &mut App,
) {
    settings.update(cx, |this, cx| {
        this.listen.draft = address;
        this.commit(TextFieldId::Listen, cx);
    });
}

fn set_server_listen_value(
    settings: &Entity<SettingsWindow>,
    address: Option<String>,
    cx: &mut App,
) {
    settings.update(cx, |this, cx| this.send_listen(address, cx));
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
    let mut group = SettingGroup::new().title("Agent integration");
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
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .items_center()
                            .child(super::sidebar::agent_mark(agent, cx.theme().foreground).small())
                            .child(agent.label()),
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
        Some(HooksState::Unsupported) => ("Unavailable", cx.theme().muted_foreground),
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
        .when(
            !matches!(state, Some(HooksState::Installed | HooksState::Unsupported)),
            |this| this.child(action("install", install_label, HooksAction::Install)),
        )
        .when(
            matches!(state, Some(HooksState::Installed | HooksState::Outdated)),
            |this| this.child(action("uninstall", "Uninstall", HooksAction::Uninstall)),
        )
        .into_any_element()
}

impl Condr {
    /// Asks a Server to store a new shell preference. The stored value comes back as a
    /// `ServerSettingsChanged` event; nothing is assumed locally. The Server persists
    /// every value it receives, so only a committed field calls this, never a keystroke.
    pub(in crate::app) fn set_server_shell(&mut self, key: ConnectionKey, shell: &str) {
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
