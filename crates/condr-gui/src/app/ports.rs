//! The Ports page (ADR 0061): forwards of remote Devices' ports to this machine. The GUI
//! process holds every listener; each Device's forwards ride one Stream connection, kept
//! by a [`StreamLink`] that follows the Device's terminal connection. The page is a page
//! of the window's content area, not a Session Tab: forwards live here and belong to no
//! Workspace.

use super::sidebar::CondrIconName;
use super::*;
use condr_client::{Forward, LinkStatus, LocalPort, StreamConnection, StreamLink, StreamOpener};
use std::sync::Arc;

/// Width of the page's Device column.
const DEVICE_COLUMN_WIDTH: Rems = rems(14.);
/// Width of the port fields in the form.
const PORT_FIELD_WIDTH: Rems = rems(8.);
/// The page repaints at most this often while forwards report changes.
const REPAINT_INTERVAL: Duration = Duration::from_millis(100);

/// Which forwards the page shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PortsScope {
    /// Every Device in the window's scope.
    All,
    Device(ConnectionKey),
}

/// One Device's forwards and the Stream connection they share.
#[derive(Default)]
pub(super) struct DeviceForwards {
    link: Option<Arc<StreamLink>>,
    entries: Vec<ForwardEntry>,
}

struct ForwardEntry {
    forward: Forward,
    /// Set when the local port is not the Device's: "localhost:3000 was in use…".
    note: Option<String>,
    _retained: Retained,
}

/// One forward's hold on the link; the last released closes the Stream connection.
struct Retained(Arc<StreamLink>);

impl Drop for Retained {
    fn drop(&mut self) {
        self.0.release();
    }
}

impl DeviceForwards {
    pub(super) fn count(&self) -> usize {
        self.entries.len()
    }

    /// The terminal connection was lost, or came back with its Bootstrap.
    pub(super) fn terminal_connected(&self, up: bool) {
        if let Some(link) = &self.link {
            link.set_terminal_connected(up);
        }
    }

    /// Disconnect, Remove and editing the Device's address end its forwards.
    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }

    fn status(&self) -> LinkStatus {
        self.link
            .as_ref()
            .map_or(LinkStatus::Idle, |link| link.status())
    }
}

/// The Ports page's form, one per GUI process.
pub(super) struct PortsForm {
    port: Entity<InputState>,
    local: Entity<InputState>,
    /// The Device the form forwards from when the page shows every Device.
    device: Option<ConnectionKey>,
    error: Option<String>,
    pending: bool,
}

impl Condr {
    /// Remote Devices a window may forward from: its own Device, or every one.
    fn forwardable(&self, scope: WindowScope) -> Vec<ConnectionKey> {
        self.connections
            .iter()
            .filter(|connection| connection.endpoint.as_local_path().is_none())
            .filter(|connection| scope.connection().is_none_or(|key| key == connection.key))
            .map(|connection| connection.key)
            .collect()
    }

    /// Forwards in a window's scope, for the sidebar's button.
    pub(super) fn forward_count(&self, scope: WindowScope) -> usize {
        self.forwardable(scope)
            .into_iter()
            .filter_map(|key| self.connection(key))
            .map(|connection| connection.forwards.count())
            .sum()
    }

    pub(super) fn open_ports(
        &mut self,
        scope: PortsScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.ports_form.is_none() {
            let port = cx.new(|cx| InputState::new(window, cx).placeholder("Port"));
            let local = cx.new(|cx| InputState::new(window, cx).placeholder("Local (same)"));
            for input in [&port, &local] {
                cx.subscribe(input, |this, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change)
                        && let Some(form) = this.ports_form.as_mut()
                    {
                        form.error = None;
                        cx.notify();
                    }
                })
                .detach();
            }
            self.ports_form = Some(PortsForm {
                port,
                local,
                device: None,
                error: None,
                pending: false,
            });
        }
        self.window_view_mut(window).ports = Some(scope);
        cx.notify();
    }

    pub(super) fn close_ports(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.window_view_mut(window).ports = None;
        cx.notify();
    }

    /// Wakes the page when a forward or a link changes, at most ten times a second.
    fn ports_events(&mut self, cx: &mut Context<Self>) -> async_channel::Sender<()> {
        if let Some(sender) = &self.ports_events {
            return sender.clone();
        }
        let (sender, receiver) = async_channel::unbounded::<()>();
        cx.spawn(async move |this, cx| {
            while receiver.recv().await.is_ok() {
                cx.background_executor().timer(REPAINT_INTERVAL).await;
                while receiver.try_recv().is_ok() {}
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    return;
                }
            }
        })
        .detach();
        self.ports_events = Some(sender.clone());
        sender
    }

    fn submit_forward(&mut self, scope: PortsScope, cx: &mut Context<Self>) {
        let Some(form) = self.ports_form.as_ref() else {
            return;
        };
        if form.pending {
            return;
        }
        let key = match scope {
            PortsScope::Device(key) => Some(key),
            PortsScope::All => form.device,
        };
        let port_text = form.port.read(cx).value().trim().to_owned();
        let local_text = form.local.read(cx).value().trim().to_owned();
        let parse = |text: &str| text.parse::<u16>().ok().filter(|port| *port != 0);
        let refuse = |this: &mut Self, error: String, cx: &mut Context<Self>| {
            if let Some(form) = this.ports_form.as_mut() {
                form.error = Some(error);
            }
            cx.notify();
        };
        let Some(key) = key else {
            return refuse(self, "Choose a Device".into(), cx);
        };
        let Some(remote) = parse(&port_text) else {
            return refuse(self, "Enter a port from 1 to 65535".into(), cx);
        };
        let local = if local_text.is_empty() {
            LocalPort::Near
        } else {
            match parse(&local_text) {
                Some(port) => LocalPort::Exact(port),
                None => return refuse(self, "Enter a local port from 1 to 65535".into(), cx),
            }
        };
        let events = self.ports_events(cx);
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        if connection.status != ConnectionStatus::Connected {
            let error = format!("{} is not connected", connection.label);
            return refuse(self, error, cx);
        }
        if connection
            .forwards
            .entries
            .iter()
            .any(|entry| entry.forward.remote_port() == remote)
        {
            let error = format!("Port {remote} on {} is already forwarded", connection.label);
            return refuse(self, error, cx);
        }
        let link = connection
            .forwards
            .link
            .get_or_insert_with(|| {
                let endpoint = connection.endpoint.for_streams();
                let changed = events.clone();
                Arc::new(StreamLink::new(
                    move || {
                        StreamConnection::connect(&endpoint, "condr-gui")
                            .map(|(connection, _)| connection)
                    },
                    move || {
                        let _ = changed.try_send(());
                    },
                ))
            })
            .clone();
        if let Some(form) = self.ports_form.as_mut() {
            form.pending = true;
            form.error = None;
        }
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    // The first forward dials here, so a Device that cannot be reached
                    // says so under the form at once.
                    link.retain()?;
                    let retained = Retained(Arc::clone(&link));
                    let opener: Arc<dyn StreamOpener> = link;
                    let forward = Forward::bind(remote, local, opener, move |_| {
                        let _ = events.try_send(());
                    })
                    .map_err(|error| error.to_string())?;
                    Ok::<_, String>((forward, retained))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(form) = this.ports_form.as_mut() {
                    form.pending = false;
                }
                match result {
                    Ok((forward, retained)) => {
                        let local = forward.local_port();
                        let note = (local != remote)
                            .then(|| format!("localhost:{remote} was in use, so {local} is used"));
                        if let Some(connection) = this.connection_mut(key) {
                            connection.forwards.entries.push(ForwardEntry {
                                forward,
                                note,
                                _retained: retained,
                            });
                        }
                    }
                    Err(error) => {
                        if let Some(form) = this.ports_form.as_mut() {
                            form.error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn stop_forward(&mut self, key: ConnectionKey, remote: u16, cx: &mut Context<Self>) {
        if let Some(connection) = self.connection_mut(key) {
            connection
                .forwards
                .entries
                .retain(|entry| entry.forward.remote_port() != remote);
        }
        cx.notify();
    }

    /// The title bar's slot where a Workspace's Tab strip would be.
    pub(super) fn render_ports_title(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .gap_2()
            .items_center()
            .px_2()
            .child(Icon::new(CondrIconName::ArrowLeftRight).size_4())
            .child(div().text_sm().child("Ports"))
            .text_color(cx.theme().foreground)
            .into_any_element()
    }

    pub(super) fn render_ports_page(
        &self,
        view: &WindowView,
        scope: PortsScope,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let devices = self.forwardable(view.scope);
        let muted = cx.theme().muted_foreground;
        let total: usize = devices
            .iter()
            .filter_map(|key| self.connection(*key))
            .map(|connection| connection.forwards.count())
            .sum();
        let close_owner = cx.weak_entity();
        let header = h_flex()
            .w_full()
            .items_center()
            .gap_3()
            .px_5()
            .py_3()
            .child(div().text_lg().child("Ports"))
            .child(
                div().text_sm().text_color(muted).child(match scope {
                    PortsScope::All => "All Devices".to_owned(),
                    PortsScope::Device(key) => self
                        .connection(key)
                        .map_or_else(String::new, |connection| connection.label.clone()),
                }),
            )
            .child(div().flex_1())
            .child(
                Button::new("close-ports")
                    .debug_selector(|| "close-ports".into())
                    .ghost()
                    .small()
                    .icon(IconName::Close)
                    .tooltip("Back to the Workspace")
                    .on_click(move |_, window, cx| {
                        let _ = close_owner.update(cx, |this, cx| this.close_ports(window, cx));
                    }),
            );

        let mut list = v_flex()
            .w(DEVICE_COLUMN_WIDTH)
            .gap_1()
            .p_2()
            .child(self.ports_scope_row(
                PortsScope::All,
                "All Devices".into(),
                total,
                None,
                scope,
                cx,
            ));
        for key in &devices {
            let Some(connection) = self.connection(*key) else {
                continue;
            };
            let state = match connection.status {
                ConnectionStatus::Connected => None,
                _ => Some("reconnecting…".to_owned()),
            };
            list = list.child(self.ports_scope_row(
                PortsScope::Device(*key),
                connection.label.clone(),
                connection.forwards.count(),
                state,
                scope,
                cx,
            ));
        }

        let shown: Vec<ConnectionKey> = match scope {
            PortsScope::All => devices.clone(),
            PortsScope::Device(key) => vec![key],
        };
        let mut detail = v_flex()
            .flex_1()
            .min_w_0()
            .gap_3()
            .p_4()
            .child(self.render_ports_form(scope, &devices, cx));
        let mut any = false;
        for key in &shown {
            let Some(connection) = self.connection(*key) else {
                continue;
            };
            for entry in &connection.forwards.entries {
                any = true;
                detail = detail.child(self.render_forward_card(connection, entry, cx));
            }
        }
        if !any {
            detail = detail.child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child("Nothing forwarded yet."),
            );
        }
        v_flex()
            .debug_selector(|| "ports-page".into())
            .size_full()
            .child(header)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_start()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(list)
                    .child(detail),
            )
            .into_any_element()
    }

    fn ports_scope_row(
        &self,
        row: PortsScope,
        label: String,
        count: usize,
        state: Option<String>,
        selected: PortsScope,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let owner = cx.weak_entity();
        let muted = cx.theme().muted_foreground;
        h_flex()
            .id(match row {
                PortsScope::All => ElementId::from("ports-scope-all"),
                PortsScope::Device(key) => ElementId::from(("ports-scope", key)),
            })
            .w_full()
            .gap_2()
            .px_2()
            .py_1()
            .rounded(cx.theme().radius)
            .when(row == selected, |row| row.bg(cx.theme().accent))
            .hover(|style| style.bg(cx.theme().accent))
            .cursor_pointer()
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_sm().truncate().child(label))
                    .when_some(state, |column, state| {
                        column.child(div().text_xs().text_color(muted).child(state))
                    }),
            )
            .child(div().text_xs().text_color(muted).child(count.to_string()))
            .on_click(move |_, window, cx| {
                let _ = owner.update(cx, |this, cx| this.open_ports(row, window, cx));
            })
            .into_any_element()
    }

    fn render_ports_form(
        &self,
        scope: PortsScope,
        devices: &[ConnectionKey],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(form) = self.ports_form.as_ref() else {
            return div().into_any_element();
        };
        let muted = cx.theme().muted_foreground;
        let mut choice = h_flex().gap_1().flex_wrap();
        if scope == PortsScope::All {
            let chosen = form.device.or_else(|| devices.first().copied());
            for key in devices {
                let Some(connection) = self.connection(*key) else {
                    continue;
                };
                let key = *key;
                let owner = cx.weak_entity();
                let button = Button::new(("ports-device", key))
                    .small()
                    .label(connection.label.clone())
                    .on_click(move |_, _, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            if let Some(form) = this.ports_form.as_mut() {
                                form.device = Some(key);
                            }
                            cx.notify();
                        });
                    });
                choice = choice.child(if chosen == Some(key) {
                    button.primary()
                } else {
                    button.ghost()
                });
            }
        }
        let submit_scope = match scope {
            PortsScope::All => form
                .device
                .or_else(|| devices.first().copied())
                .map_or(PortsScope::All, PortsScope::Device),
            device => device,
        };
        let owner = cx.weak_entity();
        v_flex()
            .gap_2()
            .when(scope == PortsScope::All && devices.is_empty(), |form| {
                form.child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child("Connect a remote Device to forward its ports."),
                )
            })
            .when(scope == PortsScope::All && !devices.is_empty(), |column| {
                column.child(choice)
            })
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .w(PORT_FIELD_WIDTH)
                            .child(Input::new(&form.port).small()),
                    )
                    .child(
                        div()
                            .w(PORT_FIELD_WIDTH)
                            .child(Input::new(&form.local).small()),
                    )
                    .child(
                        Button::new("forward-port")
                            .debug_selector(|| "forward-port".into())
                            .primary()
                            .small()
                            .label("Forward")
                            .loading(form.pending)
                            .on_click(move |_, _, cx| {
                                let _ = owner
                                    .update(cx, |this, cx| this.submit_forward(submit_scope, cx));
                            }),
                    ),
            )
            .when_some(form.error.clone(), |column, error| {
                column.child(div().text_xs().text_color(cx.theme().danger).child(error))
            })
            .into_any_element()
    }

    fn render_forward_card(
        &self,
        connection: &ServerConnection,
        entry: &ForwardEntry,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = connection.key;
        let remote = entry.forward.remote_port();
        let local = entry.forward.local_port();
        let id = forward_id(key, remote);
        let url = format!("http://localhost:{local}");
        let muted = cx.theme().muted_foreground;
        let waiting = match (connection.status, connection.forwards.status()) {
            (ConnectionStatus::Connected, LinkStatus::Connected | LinkStatus::Idle) => None,
            (ConnectionStatus::Connected, LinkStatus::Reconnecting(reason)) => {
                Some(format!("reconnecting: {reason}"))
            }
            (ConnectionStatus::Connected, LinkStatus::Refused(reason)) => Some(reason),
            _ => Some(format!("waiting for {}", connection.label)),
        };
        let connections = entry.forward.connections();
        let open_url = url.clone();
        let link_url = url.clone();
        let copy_text = format!("localhost:{local}");
        let stop_owner = cx.weak_entity();
        v_flex()
            .id(("forward-card", id))
            .gap_1()
            .p_3()
            .rounded(cx.theme().radius)
            .border_1()
            .border_color(cx.theme().border)
            .when(waiting.is_some(), |card| card.opacity(0.55))
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().text_lg().child(remote.to_string()))
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(format!("on {}", connection.label)),
                    )
                    .child(div().flex_1())
                    .child(div().text_xs().text_color(muted).child(match connections {
                        1 => "1 connection".to_owned(),
                        count => format!("{count} connections"),
                    })),
            )
            .child(
                div()
                    .id(("forward-link", id))
                    .text_sm()
                    .text_color(cx.theme().link)
                    .cursor_pointer()
                    .child(url)
                    .on_click(move |_, _, cx| cx.open_url(&link_url)),
            )
            .when_some(entry.note.clone(), |card, note| {
                card.child(div().text_xs().text_color(cx.theme().warning).child(note))
            })
            .when(entry.forward.last_refused(), |card| {
                card.child(div().text_xs().text_color(muted).child(format!(
                    "The last connection was refused: nothing listens on port {remote}."
                )))
            })
            .when_some(waiting, |card, waiting| {
                card.child(div().text_xs().text_color(muted).child(waiting))
            })
            .child(
                h_flex()
                    .gap_2()
                    .pt_1()
                    .child(
                        Button::new(("forward-open", id))
                            .small()
                            .primary()
                            .label("Open in Browser")
                            .on_click(move |_, _, cx| cx.open_url(&open_url)),
                    )
                    .child(
                        Button::new(("forward-copy", id))
                            .small()
                            .ghost()
                            .label("Copy Address")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy_text.clone()))
                            }),
                    )
                    .child(
                        Button::new(("forward-stop", id))
                            .debug_selector(move || format!("forward-stop-{key}-{remote}"))
                            .small()
                            .ghost()
                            .label("Stop")
                            .on_click(move |_, _, cx| {
                                let _ = stop_owner
                                    .update(cx, |this, cx| this.stop_forward(key, remote, cx));
                            }),
                    ),
            )
            .into_any_element()
    }

    /// The sidebar's footer button: every forward in the window's scope.
    pub(super) fn render_ports_button(
        &self,
        view: &WindowView,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let count = self.forward_count(view.scope);
        let owner = cx.weak_entity();
        let button = Button::new("open-ports")
            .debug_selector(|| "open-ports".into())
            .ghost()
            .small()
            .icon(Icon::new(CondrIconName::ArrowLeftRight))
            .tooltip("Ports")
            .accessibility_label("Ports")
            .when(view.ports == Some(PortsScope::All), |button| {
                button.selected(true)
            })
            .on_click(move |_, window, cx| {
                let _ = owner.update(cx, |this, cx| this.open_ports(PortsScope::All, window, cx));
            });
        if count == 0 {
            button.into_any_element()
        } else {
            button.label(count.to_string()).into_any_element()
        }
    }
}

/// A remote Device header's button: that Device's forwards. Free of `self`, since the
/// header is drawn from a closure the sidebar calls again on every frame.
pub(super) fn device_ports_button(
    key: ConnectionKey,
    count: usize,
    owner: WeakEntity<Condr>,
) -> AnyElement {
    let button = Button::new(("device-ports", key))
        .debug_selector(move || format!("device-ports-{key}"))
        .ghost()
        .small()
        .icon(Icon::new(CondrIconName::ArrowLeftRight))
        .tooltip("Ports on this Device")
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            let _ = owner.update(cx, |this, cx| {
                this.open_ports(PortsScope::Device(key), window, cx)
            });
        });
    if count == 0 {
        button.into_any_element()
    } else {
        button.label(count.to_string()).into_any_element()
    }
}

/// One number per forward for element ids: the Device in the high bits, the port below.
fn forward_id(key: ConnectionKey, remote: u16) -> u64 {
    key << 16 | u64::from(remote)
}

#[cfg(test)]
impl Condr {
    /// Types into the form as a person would, for tests.
    pub(super) fn fill_ports_form(
        &mut self,
        port: &str,
        local: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let form = self.ports_form.as_ref().expect("the Ports page was opened");
        form.port
            .update(cx, |input, cx| input.set_value(port.to_owned(), window, cx));
        form.local.update(cx, |input, cx| {
            input.set_value(local.to_owned(), window, cx)
        });
    }

    pub(super) fn ports_form_error(&self) -> Option<String> {
        self.ports_form.as_ref().and_then(|form| form.error.clone())
    }

    /// The local ports of a Device's forwards.
    pub(super) fn forward_local_ports(&self, key: ConnectionKey) -> Vec<u16> {
        self.connection(key).map_or_else(Vec::new, |connection| {
            connection
                .forwards
                .entries
                .iter()
                .map(|entry| entry.forward.local_port())
                .collect()
        })
    }
}
