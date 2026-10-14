use super::sidebar::CondrIconName;
use super::*;
use gpui_kit::component::setting::RenderOptions;
use gpui_kit::component::switch::Switch;

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
                .map(|connection| connection.model.settings.shell.clone().into())
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

/// What the local-network switch listens on before an address is typed: every network,
/// since a phone on the Wi-Fi cannot dial a loopback port. Noise authenticates whoever
/// connects (ADR 0011).
const DEFAULT_LISTEN: &str = "0.0.0.0:2637";

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
                    text_field_row(settings, TextFieldId::Shell, Some("".into())),
                )
                .description("Empty uses the system default."),
            ),
        )
        // Restart is the Device's own lifecycle, so it lives here for good, apart from
        // the read-only Status; Connections repeats it only while a change waits for it.
        .group(
            SettingGroup::new()
                .title("Restart")
                .item(server_restart_row(this, settings, cx)),
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
                c.model.settings.proxy.mode,
                c.running_proxy
                    .as_ref()
                    .is_some_and(|running| *running != c.model.settings.proxy),
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
                selected_connection(&settings, cx, |c| c.model.settings.proxy.mode)
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
                text_field_row(settings, TextFieldId::ProxyUrl, Some("".into())),
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
                .map(|connection| connection.model.settings.proxy.url.clone().into())
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
/// Everything about other devices reaching this one, in the order a person needs it: the
/// card a new device pairs from, the devices already paired, then how they connect.
pub(super) fn server_clients_page(
    this: &SettingsWindow,
    settings: &Entity<SettingsWindow>,
    cx: &App,
) -> SettingPage {
    SettingPage::new("Paired devices")
        .icon(IconName::Network)
        .group(pairing_group(this, settings, cx))
        .group(server_devices_group(this, settings, cx))
        .group(server_connections_group(this, settings, cx))
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
/// then how this window reaches it and whether it is connected, as a dot and a word like
/// the Device's mark in the sidebar. This machine needs no route; another Device names
/// its transport, since the address mostly repeats the name and the Edit dialog has it.
pub(super) fn server_header(
    this: &SettingsWindow,
    select: &Entity<ServerSelect>,
    cx: &App,
) -> impl IntoElement {
    let (status, route) = this
        .selected_connection(cx, |c| {
            let route = match &c.endpoint {
                Endpoint::Local(_) => None,
                Endpoint::Ssh(_) => Some("over SSH"),
                Endpoint::Tcp(_) => Some("over TCP"),
                Endpoint::P2p(_) => Some("over Peer-to-peer"),
            };
            (c.status, route)
        })
        .unwrap_or((ConnectionStatus::Disconnected, None));
    let (color, state) = match status {
        ConnectionStatus::Connected => (cx.theme().success, "Connected"),
        ConnectionStatus::Connecting => (cx.theme().warning, "Connecting"),
        ConnectionStatus::Disconnected => (cx.theme().muted_foreground, "Disconnected"),
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
            h_flex()
                .debug_selector(|| "settings-server-status".into())
                .gap_1p5()
                .items_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(
                    Icon::new(super::sidebar::CondrIconName::CircleFilled)
                        .size_2()
                        .text_color(color),
                )
                .child(state)
                .children(route),
        )
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
                    // Newest first, a repeat folded into its latest line.
                    let mut lines: Vec<(&ServerLogRecord, usize)> = Vec::new();
                    for record in errors.iter().rev() {
                        match lines.iter_mut().find(|(seen, _)| {
                            seen.level == record.level && seen.message == record.message
                        }) {
                            Some((_, repeats)) => *repeats += 1,
                            None => lines.push((record, 1)),
                        }
                    }
                    v_flex().gap_1().text_sm().children(lines.into_iter().map(
                        |(record, repeats)| {
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
                                .child(div().flex_1().min_w_0().child(record.message.clone()))
                                .when(repeats > 1, |line| {
                                    line.child(div().text_color(muted).child(format!("×{repeats}")))
                                })
                        },
                    ))
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
                .is_some_and(|running| *running != c.model.settings.proxy)
    })
    .unwrap_or(false)
}

/// Asks before restarting the selected Device's Condr, which stops every Pane on it.
fn confirm_restart(settings: &Entity<SettingsWindow>, window: &mut Window, cx: &mut App) {
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
                    "Every pane and agent on this device stops; Condr windows reconnect on \
                     their own.",
                )
                .button_props(
                    DialogButtonProps::default()
                        .ok_text("Restart")
                        .ok_variant(ButtonVariant::Danger),
                )
                .on_ok(move |_, _, cx| {
                    let _ = owner.update(cx, |owner, cx| owner.restart_server(key, cx));
                    true
                })
        });
    });
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
                            "Restart"
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
                        .on_click(move |_, window, cx| confirm_restart(&settings, window, cx)),
                )
        }),
    )
    .description(if pending {
        "Saved changes apply once Condr restarts. Every pane and agent on this device stops."
    } else {
        "Stops every pane and agent on this device."
    })
    .keywords(["restart", "apply"])
}

/// How other devices reach this one: over Peer-to-peer by its key, or over TCP at an
/// address. Saved here, applied by a restart (ADR 0038), so the
/// restart row shows only while a change waits for one.
fn server_connections_group(
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
    // No reset to defaults here: it would cut every device off at once.
    let mut group = SettingGroup::new()
        .title("Connections")
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
                ),
                locked_switch("p2p", p2p),
            )
            .description("Works from any network. No port to open.")
            .keywords(["p2p", "peer-to-peer", "relay", "remote access"]),
        )
        .item(
            reachability_item(
                "TCP",
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
                ),
                locked_switch("tcp-listener", listening),
            )
            .description("Devices dial this computer's address.")
            .keywords(["tcp", "listener", "lan", "remote access"]),
        );
    if listening {
        group = group.item(
            SettingItem::new(
                "Listen on",
                text_field_row(settings, TextFieldId::Listen, None),
            )
            .description("0.0.0.0 answers on every network.")
            .disabled(!allowed)
            .keywords(["listen", "address", "port", "tcp"]),
        );
    }
    if restart_pending(this, cx) {
        group = group.item(server_restart_row(this, settings, cx));
    }
    group
}

/// Where pairing stands on the selected Device; each state is one card (ADR 0040).
#[derive(Clone)]
enum Pairing {
    /// The Device has not reported how it is reached yet.
    Unknown,
    /// Nothing listens and nothing is saved: no device could reach this one.
    Closed {
        allowed: bool,
    },
    /// A way in is saved and waits for a restart.
    RestartPending {
        allowed: bool,
    },
    Restarting,
    /// A device could reach this one; no code is showing.
    Ready,
    Showing(ServerInvite),
    Expired,
    /// A device paired with the code that showed, by the name it gave.
    Paired(String),
}

fn pairing_state(this: &SettingsWindow, cx: &App) -> Pairing {
    let allowed = reach_allowed(this, cx);
    this.selected_connection(cx, |c| {
        if c.reconnect_deadline.is_some() {
            return Pairing::Restarting;
        }
        if let Some(name) = &c.just_paired {
            return Pairing::Paired(name.clone());
        }
        if c.health.is_none() {
            return Pairing::Unknown;
        }
        // What the process bound, not what is saved for the next start.
        if c.running_listen.is_none() && !c.running_p2p {
            return if c.listen.is_some() || c.p2p {
                Pairing::RestartPending { allowed }
            } else {
                Pairing::Closed { allowed }
            };
        }
        match &c.invite {
            Some(invite) if invite.remaining_secs() == 0 => Pairing::Expired,
            Some(invite) => Pairing::Showing(invite.clone()),
            None => Pairing::Ready,
        }
    })
    .unwrap_or(Pairing::Ready)
}

/// The white tile a phone across a desk reads the code from, and the tile that stands in
/// for it before there is one. In rems, since the column beside it, title to actions, is
/// text that must fit its height.
const PAIRING_CODE_SIDE: Rems = rems(12.);

/// The card a new device pairs from: one code to scan and its link to copy, or the one
/// step that comes before them.
fn pairing_group(
    this: &SettingsWindow,
    settings: &Entity<SettingsWindow>,
    cx: &App,
) -> SettingGroup {
    let state = pairing_state(this, cx);
    let settings = settings.clone();
    SettingGroup::new().title("Pair").item(
        SettingItem::render(move |_, window, cx| pairing_card(&state, &settings, window, cx))
            .keywords(["invite", "pair", "qr", "code", "phone", "link"]),
    )
}

fn pairing_card(
    state: &Pairing,
    settings: &Entity<SettingsWindow>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    let (tcp, copied) = {
        let this = settings.read(cx);
        (this.invite_tcp, this.invite_copied)
    };
    let side = PAIRING_CODE_SIDE.to_pixels(window.rem_size());
    // The slot the code fills, so the card keeps its shape when the code appears.
    let tile = |icon: Icon, color: Hsla, cx: &App| {
        div()
            .flex_none()
            .size(side)
            .rounded(px(8.))
            .border_1()
            .border_dashed()
            .border_color(cx.theme().border)
            .flex()
            .items_center()
            .justify_center()
            .child(icon.size(px(28.)).text_color(color))
            .into_any_element()
    };
    let show_code = |label: &'static str, primary: bool| {
        let settings = settings.clone();
        Button::new("pairing-show-code")
            .debug_selector(|| "pairing-show-code".into())
            .label(label)
            .small()
            .map(|button| {
                if primary {
                    button.primary()
                } else {
                    button.outline()
                }
            })
            .on_click(move |_, _, cx| request_invite(&settings, cx))
            .into_any_element()
    };
    use gpui_kit::assets::IconName as Glyph;
    let (code, title, body, detail, actions): (
        AnyElement,
        SharedString,
        Option<&'static str>,
        Vec<AnyElement>,
        Vec<AnyElement>,
    ) = match state {
        Pairing::Closed { allowed } => (
            tile(Icon::new(CondrIconName::QrCode), muted, cx),
            "Add a phone or computer".into(),
            Some(if *allowed {
                "Other devices can't connect to this one yet."
            } else {
                "Other devices can't connect to this one yet. Allow it on the device itself."
            }),
            Vec::new(),
            allowed
                .then(|| {
                    let settings = settings.clone();
                    Button::new("pairing-allow")
                        .debug_selector(|| "pairing-allow".into())
                        .label("Allow connections")
                        .small()
                        .primary()
                        .on_click(move |_, _, cx| set_server_p2p_enabled(&settings, true, cx))
                        .into_any_element()
                })
                .into_iter()
                .collect(),
        ),
        Pairing::RestartPending { allowed } => (
            tile(Icon::new(Glyph::RefreshCw), muted, cx),
            "Restart Condr to finish".into(),
            Some(
                "Devices can connect once Condr restarts. Every pane and agent on this device stops.",
            ),
            Vec::new(),
            allowed
                .then(|| {
                    let settings = settings.clone();
                    Button::new("pairing-restart")
                        .debug_selector(|| "pairing-restart".into())
                        .label("Restart")
                        .small()
                        .primary()
                        .on_click(move |_, window, cx| confirm_restart(&settings, window, cx))
                        .into_any_element()
                })
                .into_iter()
                .collect(),
        ),
        Pairing::Unknown => (
            tile(Icon::new(CondrIconName::QrCode), muted, cx),
            "Add a phone or computer".into(),
            None,
            Vec::new(),
            Vec::new(),
        ),
        Pairing::Restarting => (
            tile(Icon::new(Glyph::RefreshCw), muted, cx),
            "Restarting Condr…".into(),
            None,
            Vec::new(),
            Vec::new(),
        ),
        Pairing::Ready => (
            tile(Icon::new(CondrIconName::QrCode), muted, cx),
            "Add a phone or computer".into(),
            Some("Condr on your phone scans a code; another computer pastes its link."),
            Vec::new(),
            vec![show_code("Show pairing code", true)],
        ),
        Pairing::Expired => (
            tile(Icon::new(CondrIconName::QrCode), muted, cx),
            "The code expired".into(),
            Some("Show a new one when the device is ready."),
            Vec::new(),
            vec![show_code("Show a new code", true)],
        ),
        Pairing::Paired(name) => (
            tile(Icon::new(Glyph::CircleCheck), cx.theme().success, cx),
            format!("{name} is paired").into(),
            Some("It's listed under Devices and connects from now on."),
            Vec::new(),
            vec![show_code("Pair another device", false)],
        ),
        Pairing::Showing(invite) => {
            let link = invite.link(tcp);
            let code = match &link {
                Some(link) => qr::qr_code("pairing-code", link, side),
                None => tile(Icon::new(CondrIconName::QrCode), muted, cx),
            };
            let mut detail = Vec::new();
            if invite.p2p.is_some() && invite.tcp.is_some() {
                detail.push(
                    h_flex()
                        .child(pairing_route(settings, tcp, cx))
                        .into_any_element(),
                );
            }
            if invite.tcp.is_some() {
                detail.extend(invite_host_field(
                    settings,
                    invite.shows_tcp(tcp),
                    window,
                    cx,
                ));
            }
            let copy = link.clone().map(|link| {
                let settings = settings.clone();
                Button::new("pairing-copy")
                    .debug_selector(|| "pairing-copy".into())
                    .label(if copied { "Copied" } else { "Copy link" })
                    .icon(if copied { Glyph::Check } else { Glyph::Copy })
                    .small()
                    .outline()
                    .on_click(move |_, _, cx| copy_invite(&settings, link.clone(), cx))
                    .into_any_element()
            });
            let remaining = invite.remaining_secs();
            let expires = div()
                .text_xs()
                .text_color(muted)
                .child(format!(
                    "Expires in {}:{:02}",
                    remaining / 60,
                    remaining % 60
                ))
                .into_any_element();
            let new_code = {
                let settings = settings.clone();
                Button::new("pairing-new-code")
                    .label("New code")
                    .icon(Glyph::RefreshCw)
                    .small()
                    .ghost()
                    .on_click(move |_, _, cx| request_invite(&settings, cx))
                    .into_any_element()
            };
            (
                code,
                if link.is_some() {
                    "Scan with Condr on your phone".into()
                } else {
                    "Choose the address the new device dials".into()
                },
                None,
                detail,
                copy.into_iter().chain([new_code, expires]).collect(),
            )
        }
    };
    h_flex()
        .debug_selector(|| "pairing-card".into())
        .w_full()
        .gap_6()
        .items_start()
        .child(code)
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                // As tall as the tile: the title sits on its top edge and the actions on
                // its bottom one, whatever the state.
                .min_h(side)
                .gap_1()
                .child(div().font_medium().child(title))
                .children(body.map(|body| div().text_sm().text_color(muted).child(body)))
                .when(!detail.is_empty(), |column| {
                    column.child(v_flex().gap_3().pt_3().children(detail))
                })
                .when(!actions.is_empty(), |column| {
                    column.child(
                        h_flex()
                            .gap_2()
                            .mt_auto()
                            .pt_3()
                            .items_center()
                            .children(actions),
                    )
                }),
        )
        .into_any_element()
}

/// Which link the code holds when both transports are on: Peer-to-peer, the default,
/// which needs no address, or TCP at one the new device dials.
fn pairing_route(settings: &Entity<SettingsWindow>, tcp: bool, cx: &App) -> AnyElement {
    let side = |id: &'static str, label: &'static str, side_tcp: bool| {
        let settings = settings.clone();
        Button::new(id)
            .debug_selector(move || id.into())
            .label(label)
            .small()
            .map(|button| {
                if side_tcp == tcp {
                    button.primary()
                } else {
                    button.ghost()
                }
            })
            .on_click(move |_, _, cx| {
                settings.update(cx, |this, cx| {
                    this.invite_tcp = side_tcp;
                    cx.notify();
                });
            })
    };
    h_flex()
        .debug_selector(|| "pairing-route".into())
        .flex_none()
        .p_0p5()
        .gap_0p5()
        .rounded(cx.theme().radius)
        .bg(cx.theme().muted)
        .child(side("pairing-route-p2p", "Peer-to-peer", false))
        .child(side("pairing-route-tcp", "TCP", true))
        .into_any_element()
}

/// Asks the selected Device for a new invite; the card shows it when the reply lands.
fn request_invite(settings: &Entity<SettingsWindow>, cx: &mut App) {
    settings.update(cx, |this, cx| {
        let key = this.selected_server;
        this.invite_copied = false;
        let _ = this.owner.update(cx, |owner, _| {
            owner.server_admin(key, ServerAdminCommand::Invite)
        });
    });
}

/// Puts the shown link on the clipboard and says so on the button for a moment.
fn copy_invite(settings: &Entity<SettingsWindow>, link: String, cx: &mut App) {
    cx.write_to_clipboard(ClipboardItem::new_string(link));
    settings.update(cx, |this, cx| {
        this.invite_copied = true;
        cx.notify();
        this._invite_copied_reset = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(INVITE_COPIED_FOR).await;
            let _ = this.update(cx, |this, cx| {
                this.invite_copied = false;
                cx.notify();
            });
        }));
    });
}

/// How long the Copy button says "Copied".
const INVITE_COPIED_FOR: Duration = Duration::from_secs(2);

/// The TCP link's host: a field that takes any name or address, and beside it the ones
/// the Server offered, most likely first, to pick from (ADR 0040). The invite keeps the
/// host; the field shows it, whichever way it changed. While the Peer-to-peer link is the
/// one shown the field keeps its place unseen, so the actions below it stay put when the
/// route changes.
fn invite_host_field(
    settings: &Entity<SettingsWindow>,
    shown: bool,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    let (input, key, owner, (host, hosts)) = {
        let this = settings.read(cx);
        let invite = this.selected_connection(cx, |connection| {
            connection
                .invite
                .as_ref()
                .map(|invite| (invite.host.clone(), invite.hosts.clone()))
        })??;
        (
            this.invite_host.clone(),
            this.selected_server,
            this.owner.clone(),
            invite,
        )
    };
    if input.read(cx).value() != host {
        input.update(cx, |input, cx| input.set_value(host.clone(), window, cx));
    }
    let picker = Button::new("server-invite-hosts")
        .when(shown, |picker| {
            picker.debug_selector(|| "server-invite-hosts".into())
        })
        .icon(IconName::ChevronDown)
        .ghost()
        .xsmall()
        .disabled(hosts.is_empty())
        .dropdown_menu(move |menu, _, _| {
            hosts.iter().fold(
                menu.check_side(gpui_kit::component::Side::Right),
                |menu, candidate| {
                    let owner = owner.clone();
                    let candidate = candidate.clone();
                    menu.item(
                        PopupMenuItem::new(candidate.clone())
                            .checked(candidate == host.trim())
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |owner, cx| {
                                    if owner.set_invite_host(key, candidate.clone()) {
                                        cx.notify();
                                    }
                                });
                            }),
                    )
                },
            )
        });
    Some(
        v_flex()
            .gap_1()
            .when(!shown, |field| field.invisible())
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child("Reach this computer at"),
            )
            .child(
                div()
                    // Recorded even when hidden, so only an offered field carries it.
                    .when(shown, |field| {
                        field.debug_selector(|| "server-invite-host".into())
                    })
                    .w_full()
                    .max_w(rems(18.))
                    .child(Input::new(&input).small().suffix(picker)),
            )
            .into_any_element(),
    )
}

/// One row per paired device, connected ones first: its name over whether it is
/// connected or when it was last seen, the start of its fingerprint (the one `condr
/// server clients` prints) to tell two of a name apart, and Revoke.
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
                    .child("No devices yet. Pair one with the code above.")
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
                let muted = cx.theme().muted_foreground;
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
                        .child(relative_age(client.last_seen))
                        .into_any_element()
                };
                let short: String = client.fingerprint.chars().take(8).collect();
                h_flex()
                    .w_full()
                    .gap_4()
                    .items_center()
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_sm()
                                    .font_medium()
                                    .truncate()
                                    .child(client.name.clone()),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(seen)
                                    .child("·")
                                    .child(div().font_family("monospace").child(short))
                                    .when(is_this_device, |row| {
                                        row.child("·").child("This window")
                                    }),
                            ),
                    )
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
        .when(this_device, |button| {
            button.tooltip("It is how this window connects.")
        })
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
                            "It disconnects now and needs a new pairing code to connect again.",
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
        let Some(server_id) = connection.model.server_id else {
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
