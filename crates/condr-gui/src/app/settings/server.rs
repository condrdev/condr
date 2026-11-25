use super::icons::CondrIconName;
use super::*;
use crate::app::dialogs::{DangerConfirm, confirm_danger};
use crate::app::format::relative_age;
use condr_core::protocol::Uptime;
use gpui_kit::component::setting::RenderOptions;
use gpui_kit::component::switch::Switch;
use rust_i18n::t;
use std::borrow::Cow;

/// Why a TCP or Peer-to-peer window may not change how other devices reach a Device: the
/// change could cut the very connection that made it (ADR 0038).
pub(super) fn reachability_locked() -> SharedString {
    t!("settings.reachability_locked").into()
}

/// `field` while this window may change how other devices reach the Device; otherwise
/// `locked`, the same control greyed out with why on hover, since Kit's setting fields
/// take no tooltip.
fn reachability_item<F, E>(
    title: Cow<'static, str>,
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

/// What `read` makes of Device `key`'s connection, while the main window is there.
pub(super) fn read_connection<T>(
    owner: &WeakEntity<Condr>,
    key: ConnectionKey,
    cx: &App,
    read: impl FnOnce(&ServerConnection) -> T,
) -> Option<T> {
    owner.upgrade()?.read(cx).connection(key).map(read)
}

/// The shell a Server currently stores, as its Bootstrap or last event reported it.
pub(super) fn connection_shell(
    owner: &WeakEntity<Condr>,
    key: ConnectionKey,
    cx: &App,
) -> SharedString {
    read_connection(owner, key, cx, |connection| {
        connection.model.settings.shell.clone().into()
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
    SettingPage::new(t!("settings.general.title"))
        .icon(IconName::Settings2)
        .default_open(true)
        .group(server_status_group(this, cx))
        .group(
            SettingGroup::new()
                .title(t!("settings.terminal"))
                .item(
                    SettingItem::new(
                        "Shell",
                        text_field_row(settings, TextFieldId::Shell, Some("".into())),
                    )
                    .description(t!("settings.general.shell_description").to_string()),
                )
                .item(
                    SettingItem::new(
                        t!("settings.general.shell_integration"),
                        SettingField::switch(
                            {
                                let settings = settings.clone();
                                move |cx| {
                                    selected_connection(&settings, cx, |c| {
                                        c.model.settings.shell_integration
                                    })
                                    .unwrap_or_default()
                                }
                            },
                            {
                                let settings = settings.clone();
                                move |on, cx| set_shell_integration(&settings, on, cx)
                            },
                        ),
                    )
                    .description(t!("settings.general.shell_integration_description").to_string())
                    .keywords(["prompt", "osc 133", "zsh", "bash", "fish"]),
                ),
        )
        .group(
            SettingGroup::new()
                .title(t!("settings.general.startup"))
                .item(autostart_item(this, settings, cx)),
        )
        // Restart is the Device's own lifecycle, so it lives here for good, apart from
        // the read-only Status; Connections repeats it only while a change waits for it.
        .group(
            SettingGroup::new()
                .title(t!("settings.restart.title"))
                .item(server_restart_row(this, settings, cx)),
        )
}

/// What Autostart is called here (ADR 0057): Linux starts the Server at boot.
fn autostart_title() -> Cow<'static, str> {
    if cfg!(target_os = "linux") {
        t!("settings.general.autostart_boot")
    } else {
        t!("settings.general.autostart_login")
    }
}

/// Whether this machine's login item starts its Server (ADR 0057).
pub(super) fn autostart_registered() -> bool {
    condr_server::autostart::registered().is_ok_and(|path| path.is_some())
}

/// The switch for this machine's Autostart. Another Device's is its own login item,
/// which no message reaches, so its page says how to change it there.
fn autostart_item(
    this: &SettingsWindow,
    settings: &Entity<SettingsWindow>,
    cx: &App,
) -> SettingItem {
    let local = this
        .selected_connection(cx, |c| matches!(c.endpoint, Endpoint::Local(_)))
        .unwrap_or(false);
    if !local {
        return SettingItem::new(
            autostart_title(),
            SettingField::render(|_, _, cx| {
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("settings.general.autostart_remote"))
            }),
        )
        .disabled(true);
    }
    SettingItem::new(
        autostart_title(),
        SettingField::switch(
            {
                let settings = settings.clone();
                move |cx| settings.read(cx).autostart
            },
            {
                let settings = settings.clone();
                move |on, cx| set_autostart(&settings, on, cx)
            },
        ),
    )
    .description(t!("settings.general.autostart_description").to_string())
    .keywords([
        "autostart",
        "login",
        "boot",
        "startup",
        "launch agent",
        "systemd",
    ])
}

/// What flipping the Autostart switch does. Registering runs `loginctl` and `systemctl`
/// on Linux, so it happens off the UI thread; a failure is a toast (ADR 0020), and the
/// switch then shows what the login item says.
pub(in crate::app) fn set_autostart(settings: &Entity<SettingsWindow>, on: bool, cx: &mut App) {
    settings.update(cx, |this, cx| {
        this.autostart = on;
        cx.notify();
    });
    let owner = settings.read(cx).owner.clone();
    let failure = if on {
        t!(
            "settings.general.autostart_on_failed",
            title = autostart_title()
        )
    } else {
        t!(
            "settings.general.autostart_off_failed",
            title = autostart_title()
        )
    }
    .into_owned();
    let Ok(done) = owner.update(cx, |owner, cx| {
        owner.spawn_launch(
            failure,
            move || {
                if on {
                    condr_server::autostart::enable(&condr_server::resolve_server_executable()?)
                } else {
                    condr_server::autostart::disable().map(drop)
                }
            },
            cx,
        )
    }) else {
        return;
    };
    let settings = settings.downgrade();
    cx.spawn(async move |cx| {
        done.await;
        let _ = settings.update(cx, |this, cx| {
            this.autostart = autostart_registered();
            cx.notify();
        });
    })
    .detach();
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
        reachability_item(
            t!("settings.network.proxy"),
            allowed,
            mode,
            move |options| {
                Button::new("proxy-mode")
                    .label(proxy_mode_label(current))
                    .dropdown_caret(true)
                    .outline()
                    .with_size(options.size())
                    .disabled(true)
                    .tooltip(reachability_locked())
            },
        )
        .description(t!("settings.network.proxy_description").to_string())
        .keywords(["proxy", "https_proxy", "network"]),
    );
    if current == ProxyMode::Manual {
        group = group.item(
            SettingItem::new(
                t!("settings.network.proxy_url"),
                text_field_row(settings, TextFieldId::ProxyUrl, Some("".into())),
            )
            .description(t!("settings.network.proxy_url_description").to_string())
            .disabled(!allowed)
            .keywords(["proxy"]),
        );
    }
    if pending {
        group = group.item(server_restart_row(this, settings, cx));
    }
    SettingPage::new(t!("settings.network.title"))
        .icon(IconName::Globe)
        .group(group)
}

fn proxy_mode_label(mode: ProxyMode) -> Cow<'static, str> {
    match mode {
        ProxyMode::System => t!("settings.network.proxy_system"),
        ProxyMode::None => t!("settings.network.proxy_none"),
        ProxyMode::Manual => t!("settings.network.proxy_manual"),
    }
}

/// The Proxy URL a Server stores, as its Bootstrap or last event reported it.
pub(super) fn connection_proxy_url(
    owner: &WeakEntity<Condr>,
    key: ConnectionKey,
    cx: &App,
) -> SharedString {
    read_connection(owner, key, cx, |connection| {
        connection.model.settings.proxy.url.clone().into()
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

/// What flipping Shell integration does: new Panes start with it or without (ADR 0055).
pub(in crate::app) fn set_shell_integration(
    settings: &Entity<SettingsWindow>,
    on: bool,
    cx: &mut App,
) {
    settings.update(cx, |this, cx| {
        let key = this.selected_server;
        let _ = this.owner.update(cx, |owner, _| {
            owner.set_server_setting(key, ServerSetting::ShellIntegration(on))
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
    SettingPage::new(t!("settings.paired.title"))
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
        read_connection(&self.owner, self.selected_server, cx, read)
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
                Endpoint::Ssh(_) => Some(t!("settings.header.over_ssh")),
                Endpoint::Tcp(_) => Some(t!("settings.header.over_tcp")),
                Endpoint::P2p(_) => Some(t!("settings.header.over_p2p")),
            };
            (c.status, route)
        })
        .unwrap_or((ConnectionStatus::Disconnected, None));
    let (color, state) = match status {
        ConnectionStatus::Connected => (cx.theme().success, t!("settings.connected")),
        ConnectionStatus::Connecting => (cx.theme().warning, t!("settings.header.connecting")),
        ConnectionStatus::Disconnected => (
            cx.theme().muted_foreground,
            t!("settings.header.disconnected"),
        ),
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
                    Icon::new(super::icons::CondrIconName::CircleFilled)
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
    let group = SettingGroup::new().title(t!("settings.status.title"));
    let health = this.selected_connection(cx, |c| c.health.clone()).flatten();
    let Some(health) = health else {
        return group.item(SettingItem::render(|_, _, cx| {
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(t!("settings.status.waiting"))
        }));
    };
    let value =
        |text: String| SettingField::render(move |_, _, _| div().text_sm().child(text.clone()));
    let this_version = condr_core::build_identity();
    let mismatch = health.version != this_version;
    let version = health.version.clone();
    let version = SettingItem::new(
        t!("settings.status.version"),
        SettingField::render(move |_, _, cx| {
            div()
                .text_sm()
                .when(mismatch, |this| this.text_color(cx.theme().warning))
                .child(version.clone())
        }),
    )
    .keywords(["version"]);
    let version = if mismatch {
        version
            .description(t!("settings.status.version_differs", version = this_version).to_string())
    } else {
        version
    };
    let session = session_counts(&health);
    let errors = health.recent_errors;
    let mut group = group
        .item(version)
        .item(
            SettingItem::new(
                t!("settings.status.uptime"),
                value(uptime_text(health.uptime_secs).into_owned()),
            )
            .keywords(["uptime"]),
        )
        .item(SettingItem::new("Session", value(session)).keywords(["workspaces", "panes"]));
    if !errors.is_empty() {
        group = group.item(
            SettingItem::new(
                t!("settings.status.recent_errors"),
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

/// "1 workspace · 2 tabs · …" for the Session row, in the Server's report order.
fn session_counts(health: &ServerHealth) -> String {
    let (workspaces, tabs, panes, agents, clients) = (
        health.workspaces,
        health.tabs,
        health.panes,
        health.agents,
        health.clients,
    );
    [
        if workspaces == 1 {
            t!("settings.status.workspaces.one", count = workspaces)
        } else {
            t!("settings.status.workspaces.other", count = workspaces)
        },
        if tabs == 1 {
            t!("settings.status.tabs.one", count = tabs)
        } else {
            t!("settings.status.tabs.other", count = tabs)
        },
        if panes == 1 {
            t!("settings.status.panes.one", count = panes)
        } else {
            t!("settings.status.panes.other", count = panes)
        },
        if agents == 1 {
            t!("settings.status.agents.one", count = agents)
        } else {
            t!("settings.status.agents.other", count = agents)
        },
        if clients == 1 {
            t!("settings.status.clients.one", count = clients)
        } else {
            t!("settings.status.clients.other", count = clients)
        },
    ]
    .join(" · ")
}

/// The Server's uptime as `uptime_text` buckets it, in the interface's words.
fn uptime_text(secs: u64) -> Cow<'static, str> {
    match Uptime::of(secs) {
        Uptime::Seconds(seconds) => t!("time.uptime.seconds", seconds = seconds),
        Uptime::Minutes(minutes) => t!("time.uptime.minutes", minutes = minutes),
        Uptime::Hours { hours, minutes } => {
            t!("time.uptime.hours", hours = hours, minutes = minutes)
        }
        Uptime::Days {
            days,
            hours,
            minutes,
        } => t!(
            "time.uptime.days",
            days = days,
            hours = hours,
            minutes = minutes
        ),
    }
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
    confirm_danger(
        owner,
        DangerConfirm::new(
            t!("settings.restart.confirm_title"),
            t!("settings.restart.confirm_description"),
            t!("settings.restart.ok"),
        ),
        move |owner, _, cx| owner.restart_server(key, cx),
        window,
        cx,
    );
}

fn server_restart_row(
    this: &SettingsWindow,
    settings: &Entity<SettingsWindow>,
    cx: &App,
) -> SettingItem {
    let restart = settings.clone();
    let pending = restart_pending(this, cx);
    SettingItem::new(
        t!("settings.restart.row"),
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
                            .child(t!("settings.restart.waiting")),
                    )
                })
                .child(
                    Button::new("server-restart")
                        .debug_selector(|| "server-restart".into())
                        .label(if restarting {
                            t!("settings.restart.restarting")
                        } else if pending {
                            t!("settings.restart.to_apply")
                        } else {
                            t!("settings.restart.button")
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
                        .when(!allowed, |button| button.tooltip(reachability_locked()))
                        .on_click(move |_, window, cx| confirm_restart(&settings, window, cx)),
                )
        }),
    )
    .description(
        if pending {
            t!("settings.restart.pending_description")
        } else {
            t!("settings.restart.description")
        }
        .to_string(),
    )
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
                .tooltip(reachability_locked())
        }
    };
    // No reset to defaults here: it would cut every device off at once.
    let mut group = SettingGroup::new()
        .title(t!("settings.connections.title"))
        .item(
            reachability_item(
                t!("settings.connections.p2p"),
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
            .description(t!("settings.connections.p2p_description").to_string())
            .keywords(["p2p", "peer-to-peer", "relay", "remote access"]),
        )
        .item(
            reachability_item(
                "TCP".into(),
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
            .description(t!("settings.connections.tcp_description").to_string())
            .keywords(["tcp", "listener", "lan", "remote access"]),
        );
    if listening {
        group = group.item(
            SettingItem::new(
                t!("settings.connections.listen"),
                text_field_row(settings, TextFieldId::Listen, None),
            )
            .description(t!("settings.connections.listen_description").to_string())
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
    SettingGroup::new().title(t!("settings.pair.title")).item(
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
    let show_code = |label: Cow<'static, str>, primary: bool| {
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
    let (code, title, body, detail, actions): (
        AnyElement,
        SharedString,
        Option<Cow<'static, str>>,
        Vec<AnyElement>,
        Vec<AnyElement>,
    ) = match state {
        Pairing::Closed { allowed } => (
            tile(Icon::new(CondrIconName::QrCode), muted, cx),
            t!("settings.pair.add").into(),
            Some(if *allowed {
                t!("settings.pair.closed")
            } else {
                t!("settings.pair.closed_elsewhere")
            }),
            Vec::new(),
            allowed
                .then(|| {
                    let settings = settings.clone();
                    Button::new("pairing-allow")
                        .debug_selector(|| "pairing-allow".into())
                        .label(t!("settings.pair.allow"))
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
            t!("settings.pair.restart_title").into(),
            Some(t!("settings.pair.restart_body")),
            Vec::new(),
            allowed
                .then(|| {
                    let settings = settings.clone();
                    Button::new("pairing-restart")
                        .debug_selector(|| "pairing-restart".into())
                        .label(t!("settings.restart.button"))
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
            t!("settings.pair.add").into(),
            None,
            Vec::new(),
            Vec::new(),
        ),
        Pairing::Restarting => (
            tile(Icon::new(Glyph::RefreshCw), muted, cx),
            t!("settings.pair.restarting").into(),
            None,
            Vec::new(),
            Vec::new(),
        ),
        Pairing::Ready => (
            tile(Icon::new(CondrIconName::QrCode), muted, cx),
            t!("settings.pair.add").into(),
            Some(t!("settings.pair.ready")),
            Vec::new(),
            vec![show_code(t!("settings.pair.show_code"), true)],
        ),
        Pairing::Expired => (
            tile(Icon::new(CondrIconName::QrCode), muted, cx),
            t!("settings.pair.expired").into(),
            Some(t!("settings.pair.expired_body")),
            Vec::new(),
            vec![show_code(t!("settings.pair.show_new_code"), true)],
        ),
        Pairing::Paired(name) => (
            tile(Icon::new(Glyph::CircleCheck), cx.theme().success, cx),
            t!("settings.pair.paired", name = name).into(),
            Some(t!("settings.pair.paired_body")),
            Vec::new(),
            vec![show_code(t!("settings.pair.pair_another"), false)],
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
                    .label(if copied {
                        t!("settings.pair.copied")
                    } else {
                        t!("settings.pair.copy_link")
                    })
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
                .child(t!(
                    "settings.pair.expires_in",
                    time = format!("{}:{:02}", remaining / 60, remaining % 60)
                ))
                .into_any_element();
            let new_code = {
                let settings = settings.clone();
                Button::new("pairing-new-code")
                    .label(t!("settings.pair.new_code"))
                    .icon(Glyph::RefreshCw)
                    .small()
                    .ghost()
                    .on_click(move |_, _, cx| request_invite(&settings, cx))
                    .into_any_element()
            };
            (
                code,
                if link.is_some() {
                    t!("settings.pair.scan").into()
                } else {
                    t!("settings.pair.choose_address").into()
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
    let side = |id: &'static str, label: Cow<'static, str>, side_tcp: bool| {
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
        .child(side(
            "pairing-route-p2p",
            t!("settings.connections.p2p"),
            false,
        ))
        .child(side("pairing-route-tcp", "TCP".into(), true))
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
                    .child(t!("settings.pair.reach_at")),
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
    let mut group = SettingGroup::new().title(t!("settings.devices.title"));
    if clients.is_empty() {
        return group.item(
            SettingItem::render(|_, _, cx| {
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("settings.devices.empty"))
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
                            Icon::new(super::icons::CondrIconName::CircleFilled)
                                .size_2()
                                .text_color(cx.theme().success),
                        )
                        .child(t!("settings.connected"))
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
                                        row.child("·").child(t!("settings.devices.this_window"))
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
        .label(t!("settings.devices.revoke"))
        .small()
        .ghost()
        .disabled(this_device)
        .when(this_device, |button| {
            button.tooltip(t!("settings.devices.revoke_this_window"))
        })
        .on_click(move |_, window, cx| {
            let (owner, selected) = {
                let this = settings.read(cx);
                (this.owner.clone(), this.selected_server)
            };
            let key = key.clone();
            confirm_danger(
                owner,
                DangerConfirm::new(
                    t!("settings.devices.revoke_title", name = name),
                    t!("settings.devices.revoke_description"),
                    t!("settings.devices.revoke_ok"),
                ),
                move |owner, _, _| {
                    owner.server_admin(selected, ServerAdminCommand::Revoke { key: key.clone() });
                },
                window,
                cx,
            );
        })
}

/// The listen address a Server stores, or the default when it has none.
pub(super) fn connection_listen(
    owner: &WeakEntity<Condr>,
    key: ConnectionKey,
    cx: &App,
) -> SharedString {
    read_connection(owner, key, cx, |connection| connection.listen.clone())
        .flatten()
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
                    .child(t!("settings.agents.hooks_failed", error = error))
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
                            .child(super::icons::agent_mark(agent, cx.theme().foreground).small())
                            .child(agent.label()),
                    )
                    .child(agent_hooks_field(&settings, agent, report.as_ref(), cx))
            })
            .keywords([agent.label(), "hooks"]),
        );
    }
    SettingPage::new(t!("settings.agents.title"))
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
        Some(HooksState::Installed) => (t!("settings.agents.installed"), cx.theme().success),
        Some(HooksState::Outdated) => (t!("settings.agents.outdated"), cx.theme().warning),
        Some(HooksState::Missing) => (t!("settings.agents.missing"), cx.theme().muted_foreground),
        Some(HooksState::Unsupported) => (
            t!("settings.agents.unsupported"),
            cx.theme().muted_foreground,
        ),
        None => (t!("settings.agents.checking"), cx.theme().muted_foreground),
    };
    let install_label = if state == Some(HooksState::Outdated) {
        t!("settings.agents.update")
    } else {
        t!("settings.agents.install")
    };
    let action = |id: &'static str, label: Cow<'static, str>, action: HooksAction| {
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
            |this| {
                this.child(action(
                    "uninstall",
                    t!("settings.agents.uninstall"),
                    HooksAction::Uninstall,
                ))
            },
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
        let Some(connection) = self.connection_mut(key) else {
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
        if let Some(connection) = self.connection_mut(key) {
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
        if let Some(connection) = self.connection_mut(key) {
            connection.send_agent_hooks(agent, action);
        }
    }
}
