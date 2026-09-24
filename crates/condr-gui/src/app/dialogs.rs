use super::*;
use gpui_kit::component::scroll::ScrollableElement as _;

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
    /// Shows a failure the user did not cause inside a dialog: a red toast on the main
    /// window that stays until dismissed, with a Copy button so the text can be pasted
    /// into an issue. Deferred because the Window is unreachable during its own update.
    pub(super) fn report_error(&mut self, message: impl Into<String>, cx: &mut App) {
        let message = message.into();
        self.last_error = Some(message.clone());
        let message: SharedString = message.into();
        let handle = self.window_handle;
        cx.defer(move |cx| {
            let _ = handle.update(cx, |_, window, cx| {
                let copied = message.clone();
                let note = Notification::error(message.clone()).action(move |_, _, _| {
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
        self.prompt_text_input(
            title.into(),
            ok_text,
            initial,
            None,
            None,
            None,
            true,
            None,
            apply,
            window,
            cx,
        );
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
            title.into(),
            ok_text,
            String::new(),
            Some("Root directory".into()),
            Some("Absolute path on the device".into()),
            None,
            false,
            Some(key),
            apply,
            window,
            cx,
        );
    }

    fn browse_directory(&mut self, key: ConnectionKey, path: String) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        if connection.status != ConnectionStatus::Connected {
            return;
        }
        let request_id = connection.next_layout_request_id;
        connection.next_layout_request_id = request_id.wrapping_add(1).max(1);
        connection.send(ClientMessage::BrowseDirectory {
            request_id,
            path: PathBuf::from(path),
        });
        if let Some(browser) = self
            .directory_browser
            .as_mut()
            .filter(|browser| browser.key == key)
        {
            browser.request_id = request_id;
        }
    }

    pub(super) fn apply_browsed_directory(
        &mut self,
        key: ConnectionKey,
        request_id: u64,
        result: Result<BrowsedDirectory, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(browser) = self
            .directory_browser
            .as_mut()
            .filter(|browser| browser.key == key && browser.request_id == request_id)
        else {
            return;
        };
        // Only the first request goes out with the field empty; its answer names the
        // home directory, which the field then shows selected, so typing replaces it.
        if let (Ok(browsed), Some(input)) = (&result, browser.input.upgrade()) {
            let path = browsed.path.to_string_lossy().into_owned();
            let handle = self.window_handle;
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, cx| {
                    input.update(cx, |input, cx| {
                        if input.value().is_empty() {
                            input.set_value(path, window, cx);
                            input.select_all(window, cx);
                        }
                    });
                });
            });
        }
        browser.listing = Some(result);
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
        // Shown inside the field and prepended to what the user types, so a fixed scheme
        // such as `ssh://` is never typed; typing or pasting it anyway is fine.
        prefix: Option<&'static str>,
        trim_value: bool,
        // Lists the directories on this Device under what the field names, for a path.
        browse: Option<ConnectionKey>,
        apply: impl Fn(&mut Condr, String, &mut Window, &mut Context<Condr>) -> Result<(), String>
        + 'static,
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
        if let Some(key) = browse {
            self.directory_browser = Some(DirectoryBrowser {
                key,
                request_id: 0,
                input: input.downgrade(),
                listing: None,
            });
            // An emptied field keeps the last listing rather than jump back home.
            cx.subscribe(&input, move |this, input, event: &InputEvent, cx| {
                let path = input.read(cx).value().trim().to_owned();
                if matches!(event, InputEvent::Change) && !path.is_empty() {
                    this.browse_directory(key, path);
                }
            })
            .detach();
            self.browse_directory(key, String::new());
        }
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
                let content_owner = owner.clone();
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
                                .child(Input::new(&input_for_content).w_full().when_some(
                                    prefix,
                                    |input, prefix| {
                                        input.prefix(
                                            div()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(prefix),
                                        )
                                    },
                                ))
                                .children(browse.and_then(|_| {
                                    directory_browser_list(&content_owner, &input_for_content, cx)
                                }))
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
                        let value = match prefix {
                            Some(prefix) => {
                                format!("{prefix}{}", value.strip_prefix(prefix).unwrap_or(&value))
                            }
                            None => value,
                        };
                        let apply = apply.clone();
                        // A rejected value keeps the dialog and the typed text.
                        owner
                            .update(cx, |this, cx| {
                                let result = apply(this, value, window, cx);
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
            input.update(cx, |input, cx| {
                input.focus(window, cx);
                input.select_all(window, cx);
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

/// What a Root Directory dialog for a remote Device lists under its field (ADR 0002).
pub(super) struct DirectoryBrowser {
    key: ConnectionKey,
    /// The latest request; an answer to an older one is dropped.
    request_id: u64,
    input: WeakEntity<InputState>,
    listing: Option<Result<BrowsedDirectory, String>>,
}

/// The parent and subdirectories of what the field names; a click puts one in the field.
fn directory_browser_list(
    owner: &WeakEntity<Condr>,
    input: &Entity<InputState>,
    cx: &App,
) -> Option<AnyElement> {
    let condr = owner.upgrade()?;
    let browser = condr.read(cx).directory_browser.as_ref()?;
    let theme = cx.theme();
    let note = |text: String| {
        div()
            .h_7()
            .px_2()
            .flex()
            .items_center()
            .text_sm()
            .text_color(theme.muted_foreground)
            .truncate()
            .child(text)
            .into_any_element()
    };
    // A path still being typed does not exist yet: that is not an error, so it is muted.
    let browsed = match &browser.listing {
        None => return Some(note("Loading…".into())),
        Some(Err(reason)) => return Some(note(reason.clone())),
        Some(Ok(browsed)) => browsed,
    };
    let key = browser.key;
    let row = |id: String, label: String, path: String| {
        let (input, owner) = (input.clone(), owner.clone());
        h_flex()
            .id(SharedString::from(id.clone()))
            .debug_selector(move || id.clone())
            .h_7()
            .w_full()
            .px_2()
            .gap_1()
            .items_center()
            .cursor_pointer()
            .rounded(theme.radius)
            .text_sm()
            .hover(|this| this.bg(theme.list_hover))
            .on_click(move |_, window, cx| {
                input.update(cx, |input, cx| input.set_value(path.clone(), window, cx));
                let _ = owner.update(cx, |this, _| this.browse_directory(key, path.clone()));
            })
            .child(
                img(file_icons::folder_icon(theme.is_dark()))
                    .size_4()
                    .flex_shrink_0(),
            )
            .child(div().min_w_0().flex_1().truncate().child(label))
            .into_any_element()
    };
    let parent = browsed.parent.as_ref().map(|parent| {
        row(
            "directory-browser-parent".into(),
            "..".into(),
            parent.to_string_lossy().into_owned(),
        )
    });
    let directories = browsed.listing.entries.iter().map(|entry| {
        let path = absolute_path(&browsed.path, RelativePath::new(&entry.name));
        row(
            format!("directory-browser-{}", entry.name),
            entry.name.clone(),
            path.to_string_lossy().into_owned(),
        )
    });
    Some(
        v_flex()
            // Keyed by the directory, so entering one starts at its top.
            .id(SharedString::from(format!(
                "directory-browser-{}",
                browsed.path.display()
            )))
            .debug_selector(|| "directory-browser".into())
            .h_64()
            .w_full()
            .p_1()
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius)
            .overflow_y_scrollbar()
            .when(browsed.listing.truncated, |list| {
                list.child(truncated_note(cx))
            })
            .children(parent)
            .children(directories)
            .when(browsed.listing.entries.is_empty(), |list| {
                list.child(note("No subdirectories".into()))
            })
            .into_any_element(),
    )
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
