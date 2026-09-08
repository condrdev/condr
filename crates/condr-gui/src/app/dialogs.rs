use super::*;

/// What the Host field lets you type: host-name and IP characters, at most 253 of them.
/// Brackets allow a literal IPv6 address; completeness is judged on Save.
pub(super) fn host_text_is_plausible(text: &str) -> bool {
    text.chars().count() <= 253
        && text.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | ':' | '[' | ']')
        })
}

/// What the Port field lets you type: nothing yet, or a number a `u16` holds. The
/// standard parser already refuses signs, letters, spaces and anything past 65535.
pub(super) fn port_text_is_plausible(text: &str) -> bool {
    text.is_empty() || text.parse::<u16>().is_ok()
}

impl Condr {
    pub(super) fn prompt_text(
        &mut self,
        title: &'static str,
        ok_text: &'static str,
        initial: String,
        apply: impl Fn(&mut Condr, String, &mut Window, &mut Context<Condr>) -> bool + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_text_input(
            title.into(),
            ok_text,
            initial,
            None,
            None,
            true,
            apply,
            window,
            cx,
        );
    }

    pub(super) fn prompt_server_path(
        &mut self,
        title: String,
        ok_text: &'static str,
        apply: impl Fn(&mut Condr, String, &mut Window, &mut Context<Condr>) -> bool + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_text_input(
            title.into(),
            ok_text,
            String::new(),
            Some("Root directory".into()),
            Some("Absolute path on Server".into()),
            false,
            apply,
            window,
            cx,
        );
    }

    pub(super) fn prompt_directory_on(
        &mut self,
        key: ConnectionKey,
        title: String,
        ok_text: &'static str,
        apply: impl Fn(&mut Condr, PathBuf, &mut Window, &mut Context<Condr>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(local_endpoint) = self
            .connection(key)
            .map(|connection| connection.endpoint.as_local_path().is_some())
        else {
            return;
        };
        if !local_endpoint {
            self.prompt_server_path(
                title,
                ok_text,
                move |this, path, window, cx| {
                    apply(this, PathBuf::from(path), window, cx);
                    true
                },
                window,
                cx,
            );
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(title.into()),
        });
        let owner = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                let path = paths.await.ok()?.ok()??.into_iter().next()?;
                owner
                    .update_in(cx, |this, window, cx| apply(this, path, window, cx))
                    .ok()?;
                Some(())
            })
            .detach();
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prompt_text_input(
        &mut self,
        title: SharedString,
        ok_text: &'static str,
        initial: String,
        field_label: Option<SharedString>,
        placeholder: Option<SharedString>,
        trim_value: bool,
        apply: impl Fn(&mut Condr, String, &mut Window, &mut Context<Condr>) -> bool + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| {
            let input = InputState::new(window, cx).default_value(initial);
            if let Some(placeholder) = placeholder {
                input.placeholder(placeholder)
            } else {
                input
            }
        });
        let owner = cx.weak_entity();
        let apply = Rc::new(apply);
        let error = cx.new(|_| None::<String>);
        window.defer(cx, move |window, cx| {
            let input_for_content = input.clone();
            let input_for_ok = input.clone();
            let owner = owner.clone();
            let apply = apply.clone();
            window.open_dialog(cx, move |dialog, _, _| {
                let input_for_content = input_for_content.clone();
                let input_for_ok = input_for_ok.clone();
                let owner = owner.clone();
                let apply = apply.clone();
                let field_label = field_label.clone();
                let content_error = error.clone();
                let submit_error = error.clone();
                dialog
                    .title(title.clone())
                    .content(move |content, _, cx| {
                        content.child(
                            v_flex()
                                .gap_1()
                                .when_some(field_label.clone(), |field, label| {
                                    field.child(div().text_sm().child(label))
                                })
                                .child(Input::new(&input_for_content).w_full())
                                .when_some(content_error.read(cx).clone(), |field, error| {
                                    field.child(
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
                                    .label(ok_text)
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(
                                            Box::new(Confirm { secondary: false }),
                                            cx,
                                        )
                                    }),
                            ),
                    )
                    .on_ok(move |_, window, cx| {
                        let value = input_for_ok.read(cx).value().to_string();
                        let Some(value) = accepted_text_input(value, trim_value) else {
                            return false;
                        };
                        let apply = apply.clone();
                        // A rejected value keeps the dialog and the typed text.
                        owner
                            .update(cx, |this, cx| {
                                let accepted = apply(this, value, window, cx);
                                submit_error.update(cx, |error, cx| {
                                    *error = (!accepted).then(|| this.app_error.clone()).flatten();
                                    cx.notify();
                                });
                                cx.notify();
                                accepted
                            })
                            .unwrap_or(true)
                    })
            });
            input.update(cx, |input, cx| {
                input.focus(window, cx);
                input.select_all(window, cx);
            });
        });
    }

    pub(super) fn prompt_add_server(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.server_list_writable(cx) {
            return;
        }
        self.prompt_text_input(
            "Add Server".into(),
            "Add",
            String::new(),
            Some("Address".into()),
            Some("tcp://server-key[.invite]@host:port or ssh://user@host".into()),
            true,
            |this, value, _, cx| match Endpoint::parse(&value, this.device_key.as_ref()) {
                Ok(endpoint) => {
                    if this
                        .connections
                        .iter()
                        .any(|c| c.endpoint.to_string() == endpoint.to_string())
                    {
                        this.app_error = Some("Server already added".into());
                        return false;
                    }
                    this.app_error = None;
                    let key = this.next_connection_key;
                    this.next_connection_key += 1;
                    let label = match &endpoint {
                        Endpoint::Tcp(tcp) => tcp.authority(),
                        Endpoint::Ssh(ssh) => ssh.destination().to_owned(),
                        Endpoint::Local(_) => unreachable!("remote address parser"),
                    };
                    this.connections
                        .push(ServerConnection::new(key, label, endpoint));
                    this.save_servers(cx);
                    this.pending_presentation_request = None;
                    this.active_connection = key;
                    this.target_pane = None;
                    _ = this.start_connect(key);
                    true
                }
                Err(error) => {
                    this.app_error = Some(format!("Invalid server: {error}"));
                    false
                }
            },
            window,
            cx,
        );
    }

    /// Edits a remote Server's name, host and port. A TCP Server key is its identity and is
    /// shown but not editable: a different key is a different Server, added with an
    /// invite. The Local Server has nothing to edit and never gets this dialog.
    pub(super) fn prompt_edit_server_on(
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
                    .title("Edit Server")
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
                                let accepted =
                                    this.apply_server_edit(key, &name, &host, &port, window, cx);
                                submit_error.update(cx, |error, cx| {
                                    *error = (!accepted).then(|| this.app_error.clone()).flatten();
                                    cx.notify();
                                });
                                cx.notify();
                                accepted
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

    /// Applies an Edit Server dialog. A changed host or port reconnects the Server at the
    /// new address; the Server key stays. False keeps the dialog open with an error.
    pub(super) fn apply_server_edit(
        &mut self,
        key: ConnectionKey,
        name: &str,
        host: &str,
        port: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.server_list_writable(cx) {
            return false;
        }
        let Some(connection) = self.connection(key) else {
            return true;
        };
        let endpoint = match &connection.endpoint {
            Endpoint::Local(_) => return true,
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
            Err(error) => {
                self.app_error = Some(format!("Invalid server address: {error}"));
                return false;
            }
        };
        if name.is_empty() {
            self.app_error = Some("The Server needs a name".into());
            return false;
        }
        if self.connections.iter().any(|connection| {
            connection.key != key && connection.endpoint.to_string() == endpoint.to_string()
        }) {
            self.app_error = Some("Another Server already uses this address".into());
            return false;
        }
        let Some(connection) = self.connection_mut(key) else {
            return true;
        };
        connection.label = name.to_owned();
        let moved = connection.endpoint != endpoint;
        connection.endpoint = endpoint;
        let was_up = connection.status != ConnectionStatus::Disconnected;
        self.app_error = None;
        self.save_servers(cx);
        if moved && was_up {
            self.disconnect_server(key);
            if self.start_connect(key) {
                self.refresh_target_pane(self.active_connection);
                self.rebuild_dock(window, cx);
            }
        }
        true
    }

    pub(super) fn confirm_delete_server_on(
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
                    .description("This removes the Server from Condr. Its terminals keep running.")
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

    pub(super) fn prompt_rename_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.active_session() else {
            return;
        };
        let Some(workspace) = session.active_workspace() else {
            return;
        };
        self.prompt_rename_workspace_on(
            self.active_connection,
            workspace.id(),
            workspace.name().to_owned(),
            window,
            cx,
        );
    }

    pub(super) fn prompt_rename_workspace_on(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_text(
            "Rename Workspace",
            "Save",
            name,
            move |this, name, _, _| {
                this.send_layout_to(key, LayoutCommand::RenameWorkspace { workspace_id, name });
                true
            },
            window,
            cx,
        );
    }

    pub(super) fn prompt_create_worktree_on(
        &mut self,
        key: ConnectionKey,
        parent_workspace_id: WorkspaceId,
        workspace_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_text(
            "Create Worktree",
            "Create",
            default_worktree_branch(&workspace_name),
            move |this, branch, window, cx| {
                this.send_presenting_layout_to(
                    key,
                    LayoutCommand::CreateWorktree {
                        parent_workspace_id,
                        branch,
                    },
                    window,
                    cx,
                );
                true
            },
            window,
            cx,
        );
    }

    pub(super) fn choose_worktree_directory_on(
        &mut self,
        key: ConnectionKey,
        parent_workspace_id: WorkspaceId,
        workspace_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_directory_on(
            key,
            format!("Open Existing Worktree from {workspace_name}"),
            "Open",
            move |this, root_directory, window, cx| {
                this.send_presenting_layout_to(
                    key,
                    LayoutCommand::OpenWorktree {
                        parent_workspace_id,
                        root_directory,
                    },
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    pub(super) fn confirm_remove_worktree_on(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        workspace_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let owner = cx.weak_entity();
        window.defer(cx, move |window, cx| {
            window.open_alert_dialog(cx, move |alert, _, _| {
                let owner = owner.clone();
                alert
                    .title(format!("Remove worktree \"{workspace_name}\"?"))
                    .description(
                        "The worktree directory will be deleted. Modified or untracked files prevent removal; the branch is kept.",
                    )
                    .button_props(
                        DialogButtonProps::default()
                            .ok_text("Remove")
                            .ok_variant(ButtonVariant::Danger)
                            .show_cancel(true)
                            .on_ok(move |_, _, cx| {
                                owner
                                    .update(cx, |this, _| {
                                        this.send_layout_to(
                                            key,
                                            LayoutCommand::RemoveWorktree { workspace_id },
                                        )
                                    })
                                    .is_ok_and(|request_id| request_id.is_some())
                            }),
                    )
            });
        });
    }

    pub(super) fn prompt_rename_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.active_session() else {
            return;
        };
        let Some(workspace) = session.active_workspace() else {
            return;
        };
        let tab = workspace.active_tab();
        self.prompt_rename_tab_on(
            self.active_connection,
            tab.id(),
            tab.name().to_owned(),
            window,
            cx,
        );
    }

    pub(super) fn prompt_rename_tab_on(
        &mut self,
        key: ConnectionKey,
        tab_id: TabId,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_text(
            "Rename Tab",
            "Save",
            name,
            move |this, name, _, _| {
                this.send_layout_to(key, LayoutCommand::RenameTab { tab_id, name });
                true
            },
            window,
            cx,
        );
    }
}
