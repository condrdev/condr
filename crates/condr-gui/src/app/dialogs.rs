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

/// What a confirmation before an action that cannot be undone says. A name in the title
/// sits in curly quotes.
pub(in crate::app) struct DangerConfirm {
    pub(in crate::app) title: String,
    pub(in crate::app) description: String,
    /// The red button, named for the action: "Delete", not "OK".
    pub(in crate::app) ok: &'static str,
    pub(in crate::app) cancel: &'static str,
}

impl DangerConfirm {
    pub(in crate::app) fn new(
        title: impl Into<String>,
        description: impl Into<String>,
        ok: &'static str,
    ) -> Self {
        Self {
            title: title.into(),
            description: description.into(),
            ok,
            cancel: "Cancel",
        }
    }
}

/// Asks before an action that cannot be undone, then runs `on_ok` on Condr and closes.
/// It takes the owner rather than `&mut Condr` so the Settings window opens it the same way.
pub(in crate::app) fn confirm_danger(
    owner: WeakEntity<Condr>,
    confirm: DangerConfirm,
    on_ok: impl Fn(&mut Condr, &mut Window, &mut Context<Condr>) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let on_ok = Rc::new(on_ok);
    window.defer(cx, move |window, cx| {
        window.open_alert_dialog(cx, move |alert, _, _| {
            let owner = owner.clone();
            let on_ok = on_ok.clone();
            alert
                .confirm()
                .title(confirm.title.clone())
                .description(confirm.description.clone())
                .ok_text(confirm.ok)
                .ok_variant(ButtonVariant::Danger)
                .cancel_text(confirm.cancel)
                .on_ok(move |_, window, cx| {
                    let _ = owner.update(cx, |this, cx| on_ok(this, window, cx));
                    true
                })
        });
    });
}

/// The one line of text a dialog asks for. [`TextPrompt::new`] is the common case: no
/// label, trimmed, everything selected.
pub(in crate::app) struct TextPrompt {
    pub(in crate::app) title: SharedString,
    pub(in crate::app) ok_text: &'static str,
    pub(in crate::app) initial: String,
    pub(in crate::app) label: Option<SharedString>,
    pub(in crate::app) placeholder: Option<SharedString>,
    /// Shown inside the field and prepended to what the user types, so a fixed scheme
    /// such as `ssh://` is never typed; typing or pasting it anyway is fine.
    pub(in crate::app) prefix: Option<&'static str>,
    pub(in crate::app) trim: bool,
    /// Lists the directories on this Device under what the field names, for a path.
    pub(in crate::app) browse: Option<ConnectionKey>,
    /// What is selected as it opens; everything when `None`.
    pub(in crate::app) selected: Option<std::ops::Range<usize>>,
    /// Shown under the field as it opens: why the value it reopens with was refused.
    pub(in crate::app) error: Option<String>,
    /// Shown under the field when the value is blank.
    pub(in crate::app) empty: &'static str,
}

impl TextPrompt {
    pub(in crate::app) fn new(
        title: impl Into<SharedString>,
        ok_text: &'static str,
        initial: String,
    ) -> Self {
        Self {
            title: title.into(),
            ok_text,
            initial,
            label: None,
            placeholder: None,
            prefix: None,
            trim: true,
            browse: None,
            selected: None,
            error: None,
            empty: "Enter a name",
        }
    }
}

/// A dialog around a form: Cancel and `ok_text` under it, and `ok_text` or Enter runs
/// `submit`. A refusal keeps the dialog and what was typed, and lands in `error`, which
/// `content` shows under the field it is about.
pub(in crate::app) fn open_form_dialog<E: 'static>(
    title: SharedString,
    ok_text: &'static str,
    error: Entity<Option<E>>,
    content: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    submit: impl Fn(&mut Window, &mut App) -> Result<(), E> + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let content = Rc::new(content);
    let submit = Rc::new(submit);
    window.open_dialog(cx, move |dialog, _, _| {
        let (content, submit, error) = (content.clone(), submit.clone(), error.clone());
        dialog
            .title(title.clone())
            .content(move |body, window, cx| body.child(content(window, cx)))
            .footer(
                DialogFooter::new()
                    .child(
                        Button::new("dialog-cancel")
                            .debug_selector(|| "dialog-cancel".into())
                            .label("Cancel")
                            .on_click(|_, window, cx| window.dispatch_action(Box::new(Cancel), cx)),
                    )
                    .child(
                        Button::new("dialog-primary-action")
                            .debug_selector(|| "dialog-primary-action".into())
                            .primary()
                            .label(ok_text)
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(Confirm { secondary: false }), cx)
                            }),
                    ),
            )
            .on_ok(move |_, window, cx| {
                let result = submit(window, cx);
                let accepted = result.is_ok();
                error.update(cx, |error, cx| {
                    *error = result.err();
                    cx.notify();
                });
                accepted
            })
    });
}

/// A form's refusal, in red under the field it is about.
pub(in crate::app) fn field_error(error: Option<String>, cx: &App) -> Option<Div> {
    error.map(|error| {
        div()
            .debug_selector(|| "field-error".into())
            .text_sm()
            .text_color(cx.theme().danger)
            .child(error)
    })
}

impl Condr {
    /// Shows a failure the user did not cause inside a dialog: a red toast on the main
    /// window that stays until dismissed, with a Copy button so the text can be pasted
    /// into an issue. The same failure again replaces its toast instead of stacking a
    /// new one. Deferred because the Window is unreachable during its own update.
    pub(super) fn report_error(&mut self, message: impl Into<String>, cx: &mut App) {
        struct ErrorToast;
        let message = message.into();
        self.last_error = Some(message.clone());
        let message: SharedString = message.into();
        let handle = self.window_handle;
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                let copied = message.clone();
                let note = Notification::error(message.clone())
                    .id1::<ErrorToast>(message.clone())
                    .action(move |_, _, _| {
                        let text = copied.clone();
                        Button::new("copy-error")
                            .label("Copy")
                            .small()
                            .ghost()
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
                            })
                    });
                window.push_notification(note, cx);
            });
        });
    }

    pub(super) fn prompt_text(
        &mut self,
        title: &'static str,
        ok_text: &'static str,
        initial: String,
        apply: impl Fn(&mut Condr, String, &mut Window, &mut Context<Condr>) -> Result<(), String>
        + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_text_input(TextPrompt::new(title, ok_text, initial), apply, window, cx);
    }

    /// A path on `key`'s Device, typed or picked from the directories listed under the field.
    pub(super) fn prompt_server_path(
        &mut self,
        key: ConnectionKey,
        title: String,
        ok_text: &'static str,
        apply: impl Fn(&mut Condr, String, &mut Window, &mut Context<Condr>) -> Result<(), String>
        + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt_text_input(
            TextPrompt {
                label: Some("Root directory".into()),
                placeholder: Some("Absolute path on the device".into()),
                trim: false,
                browse: Some(key),
                empty: "Enter a path",
                ..TextPrompt::new(title, ok_text, String::new())
            },
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
                key,
                title,
                ok_text,
                move |this, path, window, cx| {
                    apply(this, PathBuf::from(path), window, cx);
                    Ok(())
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
                cx.update(|window, cx| {
                    owner.update(cx, |this, cx| match picked {
                        Ok(Some(paths)) => {
                            if let Some(path) = paths.into_iter().next() {
                                apply(this, path, window, cx);
                            }
                        }
                        Ok(None) => {}
                        Err(error) => {
                            // Say why, and take the path as text so work can go on.
                            this.report_error(error, cx);
                            this.prompt_server_path(
                                key,
                                title,
                                ok_text,
                                move |this, path, window, cx| {
                                    apply(this, PathBuf::from(path), window, cx);
                                    Ok(())
                                },
                                window,
                                cx,
                            );
                        }
                    })
                })
                .ok()?
                .ok()?;
                Some(())
            })
            .detach();
    }

    pub(super) fn prompt_text_input(
        &mut self,
        prompt: TextPrompt,
        apply: impl Fn(&mut Condr, String, &mut Window, &mut Context<Condr>) -> Result<(), String>
        + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let TextPrompt {
            title,
            ok_text,
            initial,
            label,
            placeholder,
            prefix,
            trim,
            browse,
            selected,
            error,
            empty,
        } = prompt;
        let input = cx.new(|cx| {
            let input = InputState::new(window, cx).default_value(initial);
            match placeholder {
                Some(placeholder) => input.placeholder(placeholder),
                None => input,
            }
        });
        if let Some(key) = browse {
            self.open_directory_browser(key, &input, cx);
        }
        let owner = cx.weak_entity();
        let error = cx.new(|_| error);
        let content = {
            let (input, owner, error) = (input.clone(), owner.clone(), error.clone());
            move |_: &mut Window, cx: &mut App| {
                let field = Input::new(&input)
                    .w_full()
                    .when_some(prefix, |field, prefix| {
                        field.prefix(div().text_color(cx.theme().muted_foreground).child(prefix))
                    });
                v_flex()
                    .gap_1()
                    .when_some(label.clone(), |column, label| {
                        column.child(div().text_sm().child(label))
                    })
                    .child(match browse {
                        Some(_) => browser_field(&owner, &input, field),
                        None => field.into_any_element(),
                    })
                    .children(browse.and_then(|_| render_directory_browser(&owner, &input, cx)))
                    .children(field_error(error.read(cx).clone(), cx))
                    .into_any_element()
            }
        };
        let submit = {
            let input = input.clone();
            move |window: &mut Window, cx: &mut App| {
                let value = input.read(cx).value().to_string();
                let Some(value) = accepted_text_input(value, trim) else {
                    return Err(empty.to_owned());
                };
                let value = match prefix {
                    Some(prefix) => {
                        format!("{prefix}{}", value.strip_prefix(prefix).unwrap_or(&value))
                    }
                    None => value,
                };
                // Enter and Create take the highlighted row, as the field says.
                let value = match (browse, owner.upgrade()) {
                    (Some(_), Some(condr)) => condr.read(cx).browser_choice(&value),
                    _ => value,
                };
                owner
                    .update(cx, |this, cx| {
                        let result = apply(this, value, window, cx);
                        cx.notify();
                        result
                    })
                    .unwrap_or(Ok(()))
            }
        };
        window.defer(cx, move |window, cx| {
            open_form_dialog(title, ok_text, error, content, submit, window, cx);
            input.update(cx, |input, cx| {
                input.focus(window, cx);
                match selected {
                    Some(range) => input.set_selected_range(range, cx),
                    None => input.select_all(window, cx),
                }
            });
        });
    }

    pub(super) fn prompt_rename_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((key, session, workspace_id, _)) = self.presented() else {
            return;
        };
        let Some(workspace) = session.workspace(workspace_id) else {
            return;
        };
        self.prompt_rename_workspace_on(
            key,
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
                Ok(())
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
        self.prompt_text_input(
            TextPrompt {
                empty: "Enter a branch name",
                ..TextPrompt::new(
                    "Create Worktree",
                    "Create",
                    default_worktree_branch(&workspace_name),
                )
            },
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
                Ok(())
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
        confirm_danger(
            cx.weak_entity(),
            DangerConfirm::new(
                format!("Remove worktree “{workspace_name}”?"),
                "The worktree directory will be deleted. Modified or untracked files prevent \
                 removal; the branch is kept.",
                "Remove",
            ),
            move |this, _, _| {
                this.send_layout_to(key, LayoutCommand::RemoveWorktree { workspace_id });
            },
            window,
            cx,
        );
    }

    pub(super) fn prompt_rename_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((key, session, _, tab_id)) = self.presented() else {
            return;
        };
        let Some(tab) = session.tab(tab_id) else {
            return;
        };
        self.prompt_rename_tab_on(key, tab.id(), tab.name().to_owned(), window, cx);
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
                Ok(())
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
