use super::super::*;
use crate::app::dialogs::{host_text_is_plausible, port_text_is_plausible};

impl Condr {
    pub(in crate::app) fn add_server_menu(
        mut menu: PopupMenu,
        owner: WeakEntity<Condr>,
    ) -> PopupMenu {
        let ssh_owner = owner.clone();
        let tcp_owner = owner;
        menu = menu.item(
            PopupMenuItem::new("SSH address")
                .icon(IconName::SquareTerminal)
                .on_click(move |_, window, cx| {
                    let _ = ssh_owner.update(cx, |this, cx| this.prompt_add_server_ssh(window, cx));
                }),
        );
        menu.item(
            PopupMenuItem::new("TCP pairing link")
                .icon(IconName::Network)
                .on_click(move |_, window, cx| {
                    let _ = tcp_owner.update(cx, |this, cx| this.prompt_add_server_tcp(window, cx));
                }),
        )
    }

    pub(in crate::app) fn prompt_add_server_ssh(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_add_server_input(
            "Connect over SSH",
            "SSH address",
            "ssh://user@host[:port]",
            Some("ssh://"),
            window,
            cx,
        );
    }

    pub(in crate::app) fn prompt_add_server_tcp(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_add_server_input(
            "Pair a TCP Device",
            "Pairing link",
            "Paste tcp://… invite link",
            Some("tcp://"),
            window,
            cx,
        );
    }

    pub(in crate::app) fn prompt_add_server(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Kept as the direct address prompt for the start page and keyboard workflow.
        // The sidebar exposes the two explicit modes through its PopupMenu.
        self.prompt_add_server_input(
            "Connect Remote Device",
            "Address",
            "tcp://server-key.invite@host:port or ssh://user@host",
            None,
            window,
            cx,
        );
    }

    fn prompt_add_server_input(
        &mut self,
        title: &'static str,
        field_label: &'static str,
        placeholder: &'static str,
        expected_scheme: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(error) = self.servers_error.clone() {
            self.report_error(error, cx);
            return;
        }
        self.prompt_text_input(
            title.into(),
            "Connect",
            String::new(),
            Some(field_label.into()),
            Some(placeholder.into()),
            true,
            move |this, value, _, cx| {
                if let Some(scheme) = expected_scheme
                    && !value.trim().to_ascii_lowercase().starts_with(scheme)
                {
                    return Err(format!("Paste a {scheme} address"));
                }
                this.add_server_from_address(&value, cx)
            },
            window,
            cx,
        );
    }

    fn add_server_from_address(
        &mut self,
        value: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        match Endpoint::parse(value, self.device_key.as_ref()) {
            Ok(endpoint) => {
                if self
                    .connections
                    .iter()
                    .any(|c| c.endpoint.to_string() == endpoint.to_string())
                {
                    return Err("This device is already added".into());
                }
                let key = self.next_connection_key;
                self.next_connection_key += 1;
                let label = match &endpoint {
                    Endpoint::Tcp(tcp) => tcp.authority(),
                    Endpoint::Ssh(ssh) => ssh.destination().to_owned(),
                    Endpoint::Local(_) => unreachable!("remote address parser"),
                };
                self.connections
                    .push(ServerConnection::new(key, label, endpoint));
                self.save_servers(cx);
                self.pending_presentation_request = None;
                self.active_connection = key;
                self.target_pane = None;
                _ = self.start_connect(key);
                Ok(())
            }
            Err(error) => Err(format!("Invalid address: {error}")),
        }
    }

    /// Edits a remote Server's name, host and port. A TCP Server key is its identity and is
    /// shown but not editable: a different key is a different Server, added with an
    /// invite. The Local Server has nothing to edit and never gets this dialog.
    pub(in crate::app) fn prompt_edit_server_on(
        &mut self,
        key: ConnectionKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((label, endpoint)) = self.connection(key).and_then(|connection| match &connection
            .endpoint
        {
            Endpoint::Local(_) => None,
            endpoint => Some((connection.label.clone(), endpoint.clone())),
        }) else {
            return;
        };
        let (host_value, port_value, server_key) = match endpoint {
            Endpoint::Tcp(tcp) => (
                tcp.host,
                tcp.port.to_string(),
                Some(tcp.server_key.to_hex()),
            ),
            Endpoint::Ssh(ssh) => (ssh.to_string(), String::new(), None),
            Endpoint::Local(_) => unreachable!(),
        };
        let ssh = server_key.is_none();
        // Typing is limited to what the field can hold: a host name or address, a port
        // number. Whether the result is complete is checked on Save.
        let name = cx.new(|cx| InputState::new(window, cx).default_value(label));
        let host = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(host_value)
                .placeholder(if ssh {
                    "ssh://user@host?bin=/opt/condr"
                } else {
                    "host or IP"
                })
                .validate(move |text, _| ssh || host_text_is_plausible(text))
        });
        let port = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(port_value)
                .placeholder(if ssh { "SSH config" } else { "port" })
                .validate(|text, _| port_text_is_plausible(text))
        });
        let fields = [
            (SharedString::from("Name"), name),
            (
                SharedString::from(if ssh { "Address" } else { "Host" }),
                host,
            ),
            (SharedString::from("Port"), port),
        ];
        // The fingerprint is shown in a read-only field: one line that scrolls sideways
        // and can be selected to compare against `condr server status`.
        let fingerprint =
            server_key.map(|key| cx.new(|cx| InputState::new(window, cx).default_value(key)));
        let owner = cx.weak_entity();
        let error = cx.new(|_| None::<String>);
        window.defer(cx, move |window, cx| {
            let inputs_for_content = fields.clone();
            let inputs_for_ok = fields.clone();
            let owner = owner.clone();
            window.open_dialog(cx, move |dialog, _, _| {
                let inputs_for_content = inputs_for_content.clone();
                let inputs_for_ok = inputs_for_ok.clone();
                let fingerprint = fingerprint.clone();
                let owner = owner.clone();
                let content_error = error.clone();
                let submit_error = error.clone();
                dialog
                    .title("Edit Remote Device")
                    .content(move |content, _, cx| {
                        let field = |(label, input): &(SharedString, Entity<InputState>)| {
                            v_flex()
                                .gap_1()
                                .child(div().text_sm().child(label.clone()))
                                .child(Input::new(input).w_full())
                        };
                        let [name, host, port] = &inputs_for_content;
                        content.child(
                            v_flex()
                                .gap_2()
                                .child(field(name))
                                // Host and port belong together, as `host:port` reads.
                                .when(ssh, |form| form.child(field(host)))
                                .when(!ssh, |form| {
                                    form.child(
                                        h_flex()
                                            .gap_2()
                                            .items_end()
                                            .child(field(host).flex_1())
                                            .child(field(port).w_24().flex_none()),
                                    )
                                })
                                .when_some(fingerprint.clone(), |form, fingerprint| {
                                    form.child(
                                        v_flex()
                                            .gap_1()
                                            .child(div().text_sm().child("Fingerprint"))
                                            .child(
                                                Input::new(&fingerprint).readonly(true).w_full(),
                                            ),
                                    )
                                })
                                .when_some(content_error.read(cx).clone(), |form, error| {
                                    form.child(
                                        div().text_sm().text_color(cx.theme().danger).child(error),
                                    )
                                }),
                        )
                    })
                    .footer(
                        DialogFooter::new()
                            .child(
                                Button::new("dialog-cancel")
                                    .debug_selector(|| "dialog-cancel".into())
                                    .label("Cancel")
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(Box::new(Cancel), cx)
                                    }),
                            )
                            .child(
                                Button::new("dialog-primary-action")
                                    .debug_selector(|| "dialog-primary-action".into())
                                    .primary()
                                    .label("Save")
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(
                                            Box::new(Confirm { secondary: false }),
                                            cx,
                                        )
                                    }),
                            ),
                    )
                    .on_ok(move |_, window, cx| {
                        let [name, host, port] = inputs_for_ok
                            .each_ref()
                            .map(|(_, input)| input.read(cx).value().trim().to_string());
                        // A rejected value keeps the dialog and the typed text.
                        owner
                            .update(cx, |this, cx| {
                                let result =
                                    this.apply_server_edit(key, &name, &host, &port, window, cx);
                                submit_error.update(cx, |error, cx| {
                                    *error = result.as_ref().err().cloned();
                                    cx.notify();
                                });
                                cx.notify();
                                result.is_ok()
                            })
                            .unwrap_or(true)
                    })
            });
            fields[0].1.update(cx, |input, cx| {
                input.focus(window, cx);
                input.select_all(window, cx);
            });
        });
    }

    /// Applies an Edit Remote Device dialog. A changed host or port reconnects the Server at the
    /// new address; the Server key stays. False keeps the dialog open with an error.
    pub(in crate::app) fn apply_server_edit(
        &mut self,
        key: ConnectionKey,
        name: &str,
        host: &str,
        port: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        if let Some(error) = self.servers_error.clone() {
            return Err(error);
        }
        let Some(connection) = self.connection(key) else {
            return Ok(());
        };
        let endpoint = match &connection.endpoint {
            Endpoint::Local(_) => return Ok(()),
            Endpoint::Tcp(tcp) => {
                TcpEndpoint::split_authority(&format!("{host}:{port}")).map(|(host, port)| {
                    Endpoint::Tcp(TcpEndpoint {
                        host,
                        port,
                        ..tcp.clone()
                    })
                })
            }
            Endpoint::Ssh(_) => condr_server::SshEndpoint::parse(host).map(Endpoint::Ssh),
        };
        let endpoint = match endpoint {
            Ok(endpoint) => endpoint,
            Err(error) => return Err(format!("Invalid address: {error}")),
        };
        if name.is_empty() {
            return Err("The device needs a name".into());
        }
        if self.connections.iter().any(|connection| {
            connection.key != key && connection.endpoint.to_string() == endpoint.to_string()
        }) {
            return Err("Another device already uses this address".into());
        }
        let Some(connection) = self.connection_mut(key) else {
            return Ok(());
        };
        connection.label = name.to_owned();
        let moved = connection.endpoint != endpoint;
        connection.endpoint = endpoint;
        let was_up = connection.status != ConnectionStatus::Disconnected;
        self.save_servers(cx);
        if moved && was_up {
            self.disconnect_server(key);
            if self.start_connect(key) {
                self.refresh_target_pane(self.active_connection);
                self.rebuild_dock(window, cx);
            }
        }
        Ok(())
    }

    pub(in crate::app) fn confirm_delete_server_on(
        &mut self,
        key: ConnectionKey,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let owner = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            window.open_alert_dialog(cx, move |alert, _, _| {
                let owner = owner.clone();
                alert
                    .title(format!("Delete \"{name}\"?"))
                    .description("This removes the device from Condr. Its terminals keep running.")
                    .button_props(
                        DialogButtonProps::default()
                            .ok_text("Delete")
                            .ok_variant(ButtonVariant::Danger)
                            .show_cancel(true)
                            .on_ok(move |_, window, cx| {
                                let _ = owner
                                    .update(cx, |this, cx| this.remove_server(key, window, cx));
                                true
                            }),
                    )
            });
        });
    }
}
