use super::*;
use gpui_kit::component::clipboard::Clipboard;
use gpui_kit::component::setting::RenderOptions;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tag::Tag;

/// Why a TCP or Peer-to-peer window may not change how other devices reach a Device: the
/// change could cut the very connection that made it (ADR 0038).
pub(super) const REACHABILITY_LOCKED: &str =
    "Change this on the device or over SSH. It can cut this connection.";

/// `field` while this window may change how other devices reach the Device; otherwise
/// `locked`, the same control greyed out with why on hover, since Kit's setting fields
/// take no tooltip.
fn reachability_item<F, E>(
    title: &'static str,
    allowed: bool,
    field: F,
    locked: impl Fn(&RenderOptions) -> E + 'static,
) -> SettingItem
where
    F: gpui_kit::component::setting::AnySettingField + 'static,
    E: IntoElement + 'static,
{
    if allowed {
        SettingItem::new(title, field)
    } else {
        SettingItem::new(
            title,
            SettingField::render(move |options, _, _| locked(options)),
        )
        .disabled(true)
    }
}

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

/// What picking a Device in the picker does; tests call it without the widget.
#[cfg(all(test, feature = "test-support"))]
pub(in crate::app) fn select_settings_server(
    settings: &Entity<SettingsWindow>,
    key: ConnectionKey,
    cx: &mut App,
) {
    settings.update(cx, |this, cx| this.select_server(key, cx));
}

/// Shows the Device's page `page_ix`; tests call it without the sidebar. A new initial
/// page starts a fresh Kit selection, so it works after the window has drawn.
#[cfg(all(test, feature = "test-support"))]
pub(in crate::app) fn select_settings_page(
    settings: &Entity<SettingsWindow>,
    page_ix: usize,
    cx: &mut App,
) {
    settings.update(cx, |this, cx| {
        this.initial_page = SelectIndex {
            page_ix,
            group_ix: None,
        };
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

/// The first of a Device's own pages: what its Server is and how it is doing, plus its
/// terminal defaults. `this` is the window being rendered, so it must not go through
/// `settings.read(cx)`.
pub(super) fn server_general_page(
    this: &SettingsWindow,
    settings: &Entity<SettingsWindow>,
    cx: &App,
) -> SettingPage {
    SettingPage::new("General")
        .icon(IconName::Settings2)
        .default_open(true)
        .group(server_status_group(this, cx))
        .group(
            SettingGroup::new().title("Terminal").item(
                SettingItem::new(
                    "Shell",
                    text_field_row(settings, TextFieldId::Shell, "".into()),
                )
                .description("Empty uses the system default."),
            ),
        )
}

/// How this machine reaches out (ADR 0038): the proxy its Server's relay connection and
/// its GUI's update check go through. Peer-to-peer takes a change when Condr restarts.
pub(super) fn network_page(
    this: &SettingsWindow,
    settings: &Entity<SettingsWindow>,
    cx: &App,
) -> SettingPage {
    let allowed = reach_allowed(this, cx);
    let (current, pending) = this
        .selected_connection(cx, |c| {
            (
                c.settings.proxy.mode,
                c.running_proxy
                    .as_ref()
                    .is_some_and(|running| *running != c.settings.proxy),
            )
        })
        .unwrap_or_default();
    let mode = SettingField::dropdown(
        ProxyMode::ALL
            .into_iter()
            .map(|mode| (mode.as_str().into(), proxy_mode_label(mode).into()))
            .collect(),
        {
            let settings = settings.clone();
            move |cx| {
                selected_connection(&settings, cx, |c| c.settings.proxy.mode)
                    .unwrap_or_default()
                    .as_str()
                    .into()
            }
        },
        {
            let settings = settings.clone();
            move |mode, cx| set_proxy_mode(&settings, ProxyMode::parse(&mode), cx)
        },
    )
    .default_value(ProxyMode::System.as_str());
    let mut group = SettingGroup::new().item(
        reachability_item("Proxy", allowed, mode, move |options| {
            Button::new("proxy-mode")
                .label(proxy_mode_label(current))
                .dropdown_caret(true)
                .outline()
                .with_size(options.size())
                .disabled(true)
                .tooltip(REACHABILITY_LOCKED)
        })
        .description("The proxy to use for network requests.")
        .keywords(["proxy", "https_proxy", "network"]),
    );
    if current == ProxyMode::Manual {
        group = group.item(
            SettingItem::new(
                "Proxy URL",
                text_field_row(settings, TextFieldId::ProxyUrl, "".into()),
            )
            .description("http:// or https://, with user:password@ if the proxy asks.")
            .disabled(!allowed)
            .keywords(["proxy"]),
        );
    }
    if pending {
        group = group.item(server_restart_row(this, settings, cx));
    }
    SettingPage::new("Network")
        .icon(IconName::Globe)
        .group(group)
}

fn proxy_mode_label(mode: ProxyMode) -> &'static str {
    match mode {
        ProxyMode::System => "System",
        ProxyMode::None => "None",
        ProxyMode::Manual => "Manual",
    }
}

/// The Proxy URL a Server stores, as its Bootstrap or last event reported it.
pub(super) fn connection_proxy_url(
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
                .map(|connection| connection.settings.proxy.url.clone().into())
        })
        .unwrap_or_default()
}

/// What choosing a proxy mode does; tests call it without the dropdown.
pub(in crate::app) fn set_proxy_mode(
    settings: &Entity<SettingsWindow>,
    mode: ProxyMode,
    cx: &mut App,
) {
    settings.update(cx, |this, cx| {
        if !reach_allowed(this, cx) {
            return;
        }
        let key = this.selected_server;
        let _ = this.owner.update(cx, |owner, _| {
            owner.set_server_setting(key, ServerSetting::ProxyMode(mode))
        });
    });
}

/// What typing a Proxy URL and leaving the field does; tests call it without the widget.
#[cfg(all(test, feature = "test-support"))]
pub(in crate::app) fn set_proxy_url(
    settings: &Entity<SettingsWindow>,
    url: SharedString,
    cx: &mut App,
) {
    settings.update(cx, |this, cx| {
        this.proxy_url.draft = url;
        this.commit(TextFieldId::ProxyUrl, cx);
    });
}

/// How other devices reach this Server: the TCP listener and Peer-to-peer, and the
/// restart that applies them.
pub(super) fn server_remote_access_page(
    this: &SettingsWindow,
    settings: &Entity<SettingsWindow>,
    cx: &App,
) -> SettingPage {
    SettingPage::new("Remote access")
        .icon(Icon::new(super::sidebar::CondrIconName::Waypoints))
        .group(server_network_group(this, settings, cx))
}

pub(super) fn server_clients_page(
    this: &SettingsWindow,
    settings: &Entity<SettingsWindow>,
    cx: &App,
) -> SettingPage {
    SettingPage::new("Paired devices")
        .icon(IconName::Network)
        .group(server_invite_group(this, settings, cx))
        .group(server_devices_group(this, settings, cx))
}

/// Whether this Client may change how other devices reach the selected Device: only a
/// local or SSH connection can, and the Server refuses the rest (ADR 0038).
fn reach_allowed(this: &SettingsWindow, cx: &App) -> bool {
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
    pub(super) fn selected_connection<T>(
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
    pub(super) fn commit_listen(&mut self, cx: &mut Context<Self>) -> CommitOutcome {
        let Ok(address) = self.listen.draft.trim().parse::<std::net::SocketAddr>() else {
            self.listen_refused = true;
            return CommitOutcome::Invalid;
        };
        self.listen_refused = false;
        let Some(listening) = self.selected_connection(cx, |c| c.listen.is_some()) else {
            return CommitOutcome::Unavailable;
        };
        if listening {
            CommitOutcome::from_queued(self.send_listen(Some(address.to_string()), cx))
        } else {
            CommitOutcome::Staged
        }
    }

    /// Sends the Proxy URL draft once it is one the Server takes; a refused draft stays in
    /// the field with the reason beside it.
    pub(super) fn commit_proxy_url(&mut self, cx: &mut Context<Self>) -> CommitOutcome {
        let url = self.proxy_url.draft.trim().to_owned();
        if let Err(error) = condr_core::check_proxy_url(&url) {
            self.proxy_url_refused = Some(error.into());
            return CommitOutcome::Invalid;
        }
        self.proxy_url_refused = None;
        if !reach_allowed(self, cx) {
            return CommitOutcome::Unavailable;
        }
        let key = self.selected_server;
        CommitOutcome::from_queued(
            self.owner
                .update(cx, |owner, _| {
                    owner.set_server_setting(key, ServerSetting::ProxyUrl(url))
                })
                .unwrap_or(false),
        )
    }

    fn send_listen(&mut self, address: Option<String>, cx: &mut Context<Self>) -> bool {
        if !reach_allowed(self, cx) {
            return false;
        }
        let key = self.selected_server;
        let command = ServerAdminCommand::SaveListen { address };
        self.owner
            .update(cx, |owner, _| owner.server_admin(key, command))
            .unwrap_or(false)
    }
}

/// The strip under the title bar that says which Device the pages describe: the picker,
/// how this window reaches it (a Server can run while this GUI is disconnected, so it
/// never claims "running") and the connection state. Only the transport: the address
/// mostly repeats the name, and the Edit dialog has it in full.
pub(super) fn server_header(
    this: &SettingsWindow,
    select: &Entity<ServerSelect>,
    cx: &App,
) -> impl IntoElement {
    let (status, route) = this
        .selected_connection(cx, |c| {
            let route = match &c.endpoint {
                Endpoint::Local(_) => "Local",
                Endpoint::Ssh(_) => "SSH",
                Endpoint::Tcp(_) => "TCP",
                Endpoint::P2p(_) => "Peer-to-peer",
            };
            (c.status, route)
        })
        .unwrap_or((ConnectionStatus::Disconnected, "Local"));
    let (tag, state) = match status {
        ConnectionStatus::Connected => (Tag::success(), "Connected"),
        ConnectionStatus::Connecting => (Tag::warning(), "Connecting"),
        ConnectionStatus::Disconnected => (Tag::secondary(), "Disconnected"),
    };
    h_flex()
        .gap_3()
        .items_center()
        .px_3()
        .py_2()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .debug_selector(|| "settings-server".into())
                .flex_none()
                .w(rems(14.))
                .child(Select::new(select).small()),
        )
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(route),
        )
        .child(tag.outline().small().child(state))
}

/// What the Server last reported about itself: version, uptime, Session counts and the
/// `warn`/`error` records it kept. Refreshed every `STATUS_REFRESH` while Settings is
/// open.
fn server_status_group(this: &SettingsWindow, cx: &App) -> SettingGroup {
    let group = SettingGroup::new().title("Status");
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
    let this_version = condr_core::build_identity();
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
            "Differs from this window, which is {this_version}."
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

/// The saved TCP listener, Peer-to-peer or proxy setting differs from what the Server
/// process started with: only a restart applies it, so the row says so and the button
/// steps forward.
fn restart_pending(this: &SettingsWindow, cx: &App) -> bool {
    this.selected_connection(cx, |c| {
        c.listen != c.running_listen
            || c.p2p != c.running_p2p
            || c.running_proxy
                .as_ref()
                .is_some_and(|running| *running != c.settings.proxy)
    })
    .unwrap_or(false)
}

fn server_restart_row(
    this: &SettingsWindow,
    settings: &Entity<SettingsWindow>,
    cx: &App,
) -> SettingItem {
    let restart = settings.clone();
    let pending = restart_pending(this, cx);
    SettingItem::new(
        "Restart Condr",
        SettingField::render(move |_, _, cx| {
            let settings = restart.clone();
            let allowed = reach_allowed(settings.read(cx), cx);
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
                        .debug_selector(|| "server-restart".into())
                        .label(if restarting {
                            "Restarting…"
                        } else if pending {
                            "Restart to apply"
                        } else {
                            "Restart Condr"
                        })
                        .small()
                        .map(|button| {
                            if pending {
                                button.primary()
                            } else {
                                button.outline()
                            }
                        })
                        .disabled(!allowed || restarting)
                        .when(!allowed, |button| button.tooltip(REACHABILITY_LOCKED))
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
    .description(if pending {
        "The changes above are saved but not applied yet. Restarting stops every pane and \
         agent on this device."
    } else {
        "Applies the changes above. Stops every pane and agent on this device."
    })
    .keywords(["restart", "apply"])
}

fn server_network_group(
    this: &SettingsWindow,
    settings: &Entity<SettingsWindow>,
    cx: &App,
) -> SettingGroup {
    let allowed = reach_allowed(this, cx);
    let (listening, p2p) = this
        .selected_connection(cx, |c| (c.listen.is_some(), c.p2p))
        .unwrap_or_default();
    let locked_switch = |id: &'static str, checked: bool| {
        move |options: &RenderOptions| {
            Switch::new(id)
                .checked(checked)
                .with_size(options.size())
                .disabled(true)
                .tooltip(REACHABILITY_LOCKED)
        }
    };
    SettingGroup::new()
        .item(reachability_item(
            "TCP listener",
            allowed,
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
            locked_switch("tcp-listener", listening),
        ))
        .item(
            SettingItem::new(
                "Listen address",
                text_field_row(settings, TextFieldId::Listen, DEFAULT_LISTEN.into()),
            )
            .description("host:port")
            .disabled(!allowed),
        )
        .item(
            reachability_item(
                "Peer-to-peer",
                allowed,
                SettingField::switch(
                    {
                        let settings = settings.clone();
                        move |cx| server_p2p_enabled(&settings, cx)
                    },
                    {
                        let settings = settings.clone();
                        move |enabled, cx| set_server_p2p_enabled(&settings, enabled, cx)
                    },
                )
                .default_value(false),
                locked_switch("p2p", p2p),
            )
            .description("Lets other devices reach this one by its key. No port to open.")
            .keywords(["p2p", "peer-to-peer", "relay"]),
        )
        .item(server_restart_row(this, settings, cx))
}

/// The invite a new device pairs with. Its rows are the Kit's, one per transport link,
/// so the page reads like the rest of Settings rather than a block of prose.
fn server_invite_group(
    this: &SettingsWindow,
    settings: &Entity<SettingsWindow>,
    cx: &App,
) -> SettingGroup {
    // Whether a device could reach this one now: what the process bound, not what is
    // saved for the next restart.
    let (reachable, pending, invite) = this
        .selected_connection(cx, |c| {
            (
                c.running_listen.is_some() || c.running_p2p,
                c.listen != c.running_listen || c.p2p != c.running_p2p,
                c.invite.clone(),
            )
        })
        .unwrap_or_default();
    // The clipboard got the p2p link, or the TCP one without it (see the `Invite` reply).
    let description: SharedString = match (&invite, reachable, pending) {
        (Some(invite), _, _) => format!(
            "The {} link is on the clipboard. Valid for {} minutes.",
            if invite.p2p.is_some() {
                "Peer-to-peer"
            } else {
                "TCP"
            },
            invite.expires_in_secs.div_ceil(60)
        )
        .into(),
        (None, false, true) => "Restart Condr to apply the Remote access changes first.".into(),
        (None, false, false) => {
            "Turn on the TCP listener or Peer-to-peer under Remote access first.".into()
        }
        (None, true, _) => "A one-time address for Connect Remote Device on the new device.".into(),
    };
    let has_invite = invite.is_some();
    let button = {
        let settings = settings.clone();
        SettingField::render(move |_, _, _| {
            let settings = settings.clone();
            Button::new("server-invite")
                .label(if has_invite {
                    "New invite"
                } else {
                    "Generate invite"
                })
                .small()
                .outline()
                .disabled(!reachable)
                .on_click(move |_, _, cx| {
                    settings.update(cx, |this, cx| {
                        let key = this.selected_server;
                        let _ = this.owner.update(cx, |owner, _| {
                            owner.server_admin(key, ServerAdminCommand::Invite)
                        });
                    });
                })
        })
    };
    let mut group = SettingGroup::new().title("Invite").item(
        SettingItem::new("Invite a device", button)
            .description(description)
            .keywords(["invite", "pair"]),
    );
    if let Some(invite) = invite {
        if let Some(address) = invite.p2p {
            group = group.item(invite_link_row(
                "Peer-to-peer",
                "server-invite-p2p",
                address,
                None,
            ));
        }
        if let Some(address) = invite.tcp {
            group = group.item(invite_link_row(
                "TCP",
                "server-invite-tcp",
                address,
                Some("Replace <host> with an address the new device can reach."),
            ));
        }
    }
    group
}

/// One transport's link under its name with its own Copy: too long for the field slot
/// beside a title, so the row stacks.
fn invite_link_row(
    title: &'static str,
    id: &'static str,
    address: String,
    description: Option<&'static str>,
) -> SettingItem {
    let item = SettingItem::new(
        title,
        SettingField::render(move |_, _, _| {
            h_flex()
                .w_full()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .debug_selector(move || id.into())
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .font_family("monospace")
                        .child(address.clone()),
                )
                .child(
                    Clipboard::new(format!("{id}-copy"))
                        .value(address.clone())
                        .tooltip("Copy invite"),
                )
        }),
    )
    .layout(Axis::Vertical)
    .keywords(["invite"]);
    match description {
        Some(description) => item.description(description),
        None => item,
    }
}

/// One row per paired device, connected ones first: the name over its fingerprint (the
/// same one `condr server clients` prints), when it was last seen, and Revoke.
fn server_devices_group(
    this: &SettingsWindow,
    settings: &Entity<SettingsWindow>,
    cx: &App,
) -> SettingGroup {
    // Over TCP or Peer-to-peer this window is one of the rows, and revoking it would cut
    // this very connection (ADR 0038).
    let this_device = (!reach_allowed(this, cx))
        .then(|| {
            let owner = this.owner.upgrade()?;
            let key = owner.read(cx).device_key.as_ref()?.public();
            Some(key.to_string())
        })
        .flatten();
    let (mut clients, connected) = this
        .selected_connection(cx, |c| (c.clients.clone(), c.connected_devices.clone()))
        .unwrap_or_default();
    let mut group = SettingGroup::new().title("Devices");
    if clients.is_empty() {
        return group.item(
            SettingItem::render(|_, _, cx| {
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child("No paired devices yet.")
            })
            .keywords(["clients", "devices"]),
        );
    }
    // Connected devices first; the Server's order (pairing order) otherwise.
    clients.sort_by_key(|client| !connected.contains(&client.fingerprint));
    for client in clients {
        let is_connected = connected.contains(&client.fingerprint);
        let is_this_device = this_device.as_ref() == Some(&client.fingerprint);
        let settings = settings.clone();
        let keywords = [SharedString::from(client.name.clone()), "revoke".into()];
        group = group.item(
            SettingItem::render(move |_, _, cx| {
                let seen: AnyElement = if is_connected {
                    h_flex()
                        .gap_1()
                        .items_center()
                        .child(
                            Icon::new(super::sidebar::CondrIconName::CircleFilled)
                                .size_2()
                                .text_color(cx.theme().success),
                        )
                        .child("Connected")
                        .into_any_element()
                } else {
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(relative_age(client.last_seen))
                        .into_any_element()
                };
                h_flex()
                    .w_full()
                    .gap_4()
                    .items_center()
                    .text_sm()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(client.name.clone())
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .font_family("monospace")
                                    .truncate()
                                    .child(client.fingerprint.clone()),
                            ),
                    )
                    .child(div().whitespace_nowrap().child(seen))
                    .child(revoke_button(&settings, &client, is_this_device))
            })
            .keywords(keywords),
        );
    }
    group
}

fn revoke_button(
    settings: &Entity<SettingsWindow>,
    client: &ServerClientInfo,
    this_device: bool,
) -> Button {
    let settings = settings.clone();
    let key = client.fingerprint.clone();
    let name = client.name.clone();
    Button::new(format!("revoke-{key}"))
        .label("Revoke")
        .small()
        .ghost()
        .disabled(this_device)
        .when(this_device, |button| button.tooltip("This device"))
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
                        .title(format!("Revoke \u{201c}{name}\u{201d}?"))
                        .description(
                            "Its connections close now, and it needs a new invite to pair again.",
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
                                    ServerAdminCommand::Revoke { key: key.clone() },
                                )
                            });
                            true
                        })
                });
            });
        })
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

fn server_p2p_enabled(settings: &Entity<SettingsWindow>, cx: &App) -> bool {
    selected_connection(settings, cx, |c| c.p2p).unwrap_or(false)
}

/// Saves `[server.p2p] enabled` on the selected Server; the restart row applies it.
fn set_server_p2p_enabled(settings: &Entity<SettingsWindow>, enabled: bool, cx: &mut App) {
    settings.update(cx, |this, cx| {
        if !reach_allowed(this, cx) {
            return;
        }
        let key = this.selected_server;
        let _ = this.owner.update(cx, |owner, _| {
            owner.server_admin(key, ServerAdminCommand::SaveP2p { enabled })
        });
    });
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
    let mut group = SettingGroup::new();
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
    SettingPage::new("Agent integrations")
        .icon(IconName::Bot)
        .group(group)
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
        Some(HooksState::Unsupported) => ("Recognition only", cx.theme().muted_foreground),
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
    /// Asks a Server to store one setting. The stored value comes back as a
    /// `ServerSettingsChanged` event; nothing is assumed locally. The Server persists
    /// every value it receives, so only a committed field calls this, never a keystroke.
    pub(in crate::app) fn set_server_setting(
        &mut self,
        key: ConnectionKey,
        setting: ServerSetting,
    ) -> bool {
        let Some(connection) = self
            .connections
            .iter_mut()
            .find(|connection| connection.key == key)
        else {
            return false;
        };
        let Some(server_id) = connection.server_id else {
            return false;
        };
        connection.send(ClientMessage::SetServerSettings { server_id, setting })
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
