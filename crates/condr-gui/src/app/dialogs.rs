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
            prompt: Some(title.clone().into()),
        });
        let owner = cx.weak_entity();
        window
            .spawn(cx, async move |cx| {
                // `Ok(None)` is the user cancelling; an error means the native picker
                // never opened (on Linux: no xdg-desktop-portal), which the platform
                // explains in its message.
                let picked = match paths.await {
                    Ok(Ok(picked)) => Ok(picked),
                    Ok(Err(error)) => Err(error.to_string()),
                    Err(_) => Err("The folder picker closed without a result".to_string()),
                };
                owner
                    .update_in(cx, |this, window, cx| match picked {
                        Ok(Some(paths)) => {
                            if let Some(path) = paths.into_iter().next() {
                                apply(this, path, window, cx);
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            // Say why, and take the path as text so work can go on.
                            this.app_error = Some(error);
                            cx.notify();
                            this.prompt_server_path(
                                title,
                                ok_text,
                                move |this, path, window, cx| {
                                    apply(this, PathBuf::from(path), window, cx);
                                    true
                                },
                                window,
                                cx,
                            );
                        }
                    })
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

pub(super) fn accepted_text_input(value: String, trim_value: bool) -> Option<String> {
    if value.trim().is_empty() {
        return None;
    }
    Some(if trim_value {
        value.trim().to_owned()
    } else {
        value
    })
}
