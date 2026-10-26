//! Creating, renaming and deleting entries from the Files and Changes views (ADR 0045),
//! and showing one in this machine's file manager.

use super::dialogs::{DangerConfirm, confirm_danger};
use super::*;
use condr_core::{FileKind, FileMutation, FileMutationResult};

/// A create, rename or delete in flight.
pub(super) enum EntryRequest {
    Create {
        workspace_id: WorkspaceId,
        path: RelativePathBuf,
        kind: FileKind,
    },
    Rename {
        workspace_id: WorkspaceId,
        from: RelativePathBuf,
        to: RelativePathBuf,
        kind: FileKind,
    },
    Delete {
        path: RelativePathBuf,
    },
}

/// Why `name` cannot be given to a new or renamed entry, as far as this Client can tell;
/// the Server checks again and knows its own platform's rules.
pub(super) fn entry_name_problem(name: &str, taken: impl Fn(&str) -> bool) -> Option<&'static str> {
    if name.is_empty() {
        Some("Enter a name")
    } else if name.contains(['/', '\\']) {
        Some("A name cannot hold / or \\")
    } else if name == "." || name == ".." {
        Some("Not a name")
    } else if name.eq_ignore_ascii_case(".git") {
        Some("Condr does not change .git")
    } else if taken(name) {
        Some("Already exists")
    } else {
        None
    }
}

/// The part of a file name before its extension, which a rename selects first.
pub(super) fn name_stem(name: &str, kind: FileKind) -> std::ops::Range<usize> {
    match (kind, name.rfind('.')) {
        (FileKind::File, Some(dot)) if dot > 0 => 0..dot,
        _ => 0..name.len(),
    }
}

/// "Reveal in Finder" and its counterparts; Linux opens the folder without selecting.
pub(super) fn reveal_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Reveal in Finder"
    } else if cfg!(windows) {
        "Reveal in File Explorer"
    } else {
        "Open Containing Folder"
    }
}

fn parent_of(path: &RelativePath) -> RelativePathBuf {
    path.parent()
        .map(RelativePath::to_relative_path_buf)
        .unwrap_or_default()
}

impl Condr {
    /// Whether a rename this Client sent may move `path`: the Tab showing it waits for the
    /// answer before it treats the new path as another file.
    pub(super) fn rename_in_flight(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: &RelativePath,
    ) -> bool {
        self.files_view
            .entry_requests
            .iter()
            .any(|((request_key, _), request)| {
                *request_key == key
                    && matches!(request, EntryRequest::Rename { workspace_id: id, from, .. }
                        if *id == workspace_id && path.starts_with(from))
            })
    }

    pub(super) fn forget_entry_requests(&mut self, key: ConnectionKey) {
        self.files_view
            .entry_requests
            .retain(|(request_key, _), _| *request_key != key);
    }

    fn send_entry(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        request: EntryRequest,
        cx: &mut App,
    ) {
        let mutation = match &request {
            EntryRequest::Create { path, kind, .. } => FileMutation::Create {
                path: path.clone(),
                kind: *kind,
            },
            EntryRequest::Rename { from, to, .. } => FileMutation::Rename {
                from: from.clone(),
                to: to.clone(),
            },
            EntryRequest::Delete { path } => FileMutation::Delete { path: path.clone() },
        };
        if let Some(request_id) = self.send_file_mutation(key, workspace_id, mutation, cx) {
            self.files_view
                .entry_requests
                .insert((key, request_id), request);
        }
    }

    pub(super) fn entry_mutated(
        &mut self,
        key: ConnectionKey,
        request_id: u64,
        result: FileMutationResult,
        cx: &mut Context<Self>,
    ) {
        let Some(request) = self.files_view.entry_requests.remove(&(key, request_id)) else {
            return;
        };
        let error = match result {
            FileMutationResult::Done { .. } => {
                self.entry_done(key, request);
                cx.notify();
                return;
            }
            // A name the Server refused goes back under its field (ADR 0020).
            FileMutationResult::Invalid(reason) => match request {
                EntryRequest::Create {
                    workspace_id,
                    path,
                    kind,
                } => {
                    self.files_view
                        .deferred
                        .push(Box::new(move |this, window, cx| {
                            let name = path.file_name().unwrap_or_default().to_owned();
                            this.prompt_new_entry(
                                key,
                                workspace_id,
                                parent_of(&path),
                                kind,
                                name,
                                Some(reason),
                                window,
                                cx,
                            );
                        }));
                    cx.notify();
                    return;
                }
                EntryRequest::Rename {
                    workspace_id,
                    from,
                    to,
                    kind,
                } => {
                    self.files_view
                        .deferred
                        .push(Box::new(move |this, window, cx| {
                            let name = to.file_name().unwrap_or_default().to_owned();
                            this.prompt_rename_entry(
                                key,
                                workspace_id,
                                from,
                                kind,
                                name,
                                Some(reason),
                                window,
                                cx,
                            );
                        }));
                    cx.notify();
                    return;
                }
                EntryRequest::Delete { path } => format!("Couldn't delete {path}: {reason}"),
            },
            FileMutationResult::Failed(reason) => match request {
                EntryRequest::Create { path, .. } => format!("Couldn't create {path}: {reason}"),
                EntryRequest::Rename { from, .. } => format!("Couldn't rename {from}: {reason}"),
                EntryRequest::Delete { path } => format!("Couldn't delete {path}: {reason}"),
            },
            FileMutationResult::Conflict => return,
        };
        self.report_error(error, cx);
    }

    fn entry_done(&mut self, key: ConnectionKey, request: EntryRequest) {
        match request {
            EntryRequest::Create {
                workspace_id,
                path,
                kind,
            } => {
                let parent = parent_of(&path);
                if !parent.as_str().is_empty() {
                    self.files_view
                        .expanded_dirs
                        .insert((key, workspace_id, parent));
                }
                if kind == FileKind::File {
                    // A new file opens in Source with the cursor in it.
                    self.files_view.pending_file_line = Some((key, workspace_id, path.clone(), 0));
                    self.files_view
                        .deferred
                        .push(Box::new(move |this, window, cx| {
                            this.show_file_on(key, workspace_id, path, window, cx);
                        }));
                }
            }
            EntryRequest::Rename {
                workspace_id,
                from,
                to,
                ..
            } => {
                let moved = |path: &RelativePath| {
                    path.strip_prefix(&from).ok().map(|rest| {
                        if rest.as_str().is_empty() {
                            to.clone()
                        } else {
                            to.join(rest)
                        }
                    })
                };
                // The Server moved the Preview Tab; its text and folds follow here.
                for ((editor_key, _), editor) in &mut self.files_view.file_editors {
                    if *editor_key == key
                        && editor.workspace_id == workspace_id
                        && let Some((shown, generation)) = &mut editor.shown
                        && let Some(path) = moved(shown)
                    {
                        *shown = path;
                        // Read the new path afresh; its version tells whether it changed.
                        *generation = 0;
                    }
                }
                let expanded = std::mem::take(&mut self.files_view.expanded_dirs);
                self.files_view.expanded_dirs = expanded
                    .into_iter()
                    .map(|(dir_key, id, path)| {
                        let path = (dir_key == key && id == workspace_id)
                            .then(|| moved(&path))
                            .flatten()
                            .unwrap_or(path);
                        (dir_key, id, path)
                    })
                    .collect();
            }
            EntryRequest::Delete { .. } => {}
        }
    }

    /// A dialog that takes one name, keeps it while it is refused, and selects `selected`.
    #[allow(clippy::too_many_arguments)]
    fn prompt_entry_name(
        &mut self,
        title: String,
        ok_text: &'static str,
        initial: String,
        selected: std::ops::Range<usize>,
        error: Option<String>,
        submit: impl Fn(&mut Condr, String, &mut Window, &mut Context<Condr>) -> Result<(), String>
        + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| InputState::new(window, cx).default_value(initial));
        let error = cx.new(|_| error);
        let owner = cx.weak_entity();
        let submit = Rc::new(submit);
        window.defer(cx, move |window, cx| {
            let (dialog_input, dialog_error) = (input.clone(), error.clone());
            window.open_dialog(cx, move |dialog, _, _| {
                let (content_input, ok_input) = (dialog_input.clone(), dialog_input.clone());
                let (content_error, ok_error) = (dialog_error.clone(), dialog_error.clone());
                let (owner, submit) = (owner.clone(), submit.clone());
                dialog
                    .title(title.clone())
                    .content(move |content, _, cx| {
                        content.child(
                            v_flex()
                                .gap_1()
                                .child(Input::new(&content_input).w_full())
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
                        let value = ok_input.read(cx).value().to_string();
                        let submit = submit.clone();
                        let result = owner
                            .update(cx, |this, cx| submit(this, value, window, cx))
                            .unwrap_or(Ok(()));
                        ok_error.update(cx, |error, cx| {
                            *error = result.as_ref().err().cloned();
                            cx.notify();
                        });
                        result.is_ok()
                    })
            });
            input.update(cx, |input, cx| {
                input.focus(window, cx);
                input.set_selected_range(selected, cx);
            });
        });
    }

    /// Whether the cached listing of `directory` already holds `name`.
    fn listed(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        directory: &RelativePath,
        name: &str,
    ) -> bool {
        self.connection(key)
            .and_then(|connection| {
                connection
                    .resources
                    .directories
                    .get(&(workspace_id, directory.to_relative_path_buf()))
            })
            .and_then(|listing| listing.as_ref().ok())
            .is_some_and(|listing| listing.entries.iter().any(|entry| entry.name == name))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prompt_new_entry(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        parent: RelativePathBuf,
        kind: FileKind,
        initial: String,
        error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let title = match (kind, parent.as_str()) {
            (FileKind::File, "") => "New File".to_owned(),
            (FileKind::Directory, "") => "New Folder".to_owned(),
            (FileKind::File, parent) => format!("New File in {parent}"),
            (FileKind::Directory, parent) => format!("New Folder in {parent}"),
        };
        let selected = 0..initial.len();
        self.prompt_entry_name(
            title,
            "Create",
            initial,
            selected,
            error,
            move |this, name, _, cx| {
                if let Some(problem) =
                    entry_name_problem(&name, |name| this.listed(key, workspace_id, &parent, name))
                {
                    return Err(problem.into());
                }
                let path = parent.join(&name);
                this.send_entry(
                    key,
                    workspace_id,
                    EntryRequest::Create {
                        workspace_id,
                        path,
                        kind,
                    },
                    cx,
                );
                Ok(())
            },
            window,
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prompt_rename_entry(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        from: RelativePathBuf,
        kind: FileKind,
        initial: String,
        error: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let current = from.file_name().unwrap_or_default().to_owned();
        let selected = name_stem(&initial, kind);
        self.prompt_entry_name(
            format!("Rename {from}"),
            "Rename",
            initial,
            selected,
            error,
            move |this, name, _, cx| {
                if name == current {
                    return Ok(());
                }
                let parent = parent_of(&from);
                if let Some(problem) =
                    entry_name_problem(&name, |name| this.listed(key, workspace_id, &parent, name))
                {
                    return Err(problem.into());
                }
                this.send_entry(
                    key,
                    workspace_id,
                    EntryRequest::Rename {
                        workspace_id,
                        from: from.clone(),
                        to: parent.join(&name),
                        kind,
                    },
                    cx,
                );
                Ok(())
            },
            window,
            cx,
        );
    }

    pub(super) fn confirm_delete_entry(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: RelativePathBuf,
        kind: FileKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let description = match kind {
            FileKind::Directory => {
                format!("{path} and everything in it will be deleted permanently.")
            }
            FileKind::File => format!("{path} will be deleted permanently."),
        };
        let title = format!("Delete “{}”?", path.file_name().unwrap_or(path.as_str()));
        confirm_danger(
            cx.weak_entity(),
            DangerConfirm::new(title, description, "Delete"),
            move |this, _, cx| {
                let path = path.clone();
                this.send_entry(key, workspace_id, EntryRequest::Delete { path }, cx);
            },
            window,
            cx,
        );
    }

    /// The edits a row's menu offers below its read-only actions: New File and New Folder
    /// in the Files view (`create_in` names where), then Rename and Delete, which a row
    /// Changes shows as deleted cannot take.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn entry_edit_items(
        menu: PopupMenu,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: &RelativePath,
        kind: FileKind,
        create_in: Option<RelativePathBuf>,
        exists: bool,
        owner: WeakEntity<Self>,
    ) -> PopupMenu {
        let menu = match create_in {
            Some(parent) => {
                let (file_owner, folder_owner) = (owner.clone(), owner.clone());
                let file_parent = parent.clone();
                menu.item(
                    PopupMenuItem::new("New File…").on_click(move |_, window, cx| {
                        let parent = file_parent.clone();
                        let _ = file_owner.update(cx, |this, cx| {
                            this.prompt_new_entry(
                                key,
                                workspace_id,
                                parent,
                                FileKind::File,
                                String::new(),
                                None,
                                window,
                                cx,
                            )
                        });
                    }),
                )
                .item(
                    PopupMenuItem::new("New Folder…").on_click(move |_, window, cx| {
                        let parent = parent.clone();
                        let _ = folder_owner.update(cx, |this, cx| {
                            this.prompt_new_entry(
                                key,
                                workspace_id,
                                parent,
                                FileKind::Directory,
                                String::new(),
                                None,
                                window,
                                cx,
                            )
                        });
                    }),
                )
                .separator()
            }
            None => menu,
        };
        let (rename_path, delete_path) = (path.to_relative_path_buf(), path.to_relative_path_buf());
        let rename_owner = owner.clone();
        menu.item(
            PopupMenuItem::new("Rename…")
                .disabled(!exists)
                .on_click(move |_, window, cx| {
                    let from = rename_path.clone();
                    let name = from.file_name().unwrap_or_default().to_owned();
                    let _ = rename_owner.update(cx, |this, cx| {
                        this.prompt_rename_entry(
                            key,
                            workspace_id,
                            from,
                            kind,
                            name,
                            None,
                            window,
                            cx,
                        )
                    });
                }),
        )
        .item(
            PopupMenuItem::new("Delete…")
                .disabled(!exists)
                .on_click(move |_, window, cx| {
                    let path = delete_path.clone();
                    let _ = owner.update(cx, |this, cx| {
                        this.confirm_delete_entry(key, workspace_id, path, kind, window, cx)
                    });
                }),
        )
    }

    /// Shows an entry of a local Workspace selected in this machine's file manager; a
    /// failure shows as a notification, as "Open in" does.
    pub(super) fn reveal_entry(&self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |_, cx| {
            let result = cx
                .background_spawn({
                    let path = path.clone();
                    async move { open_in::reveal_entry(&path) }
                })
                .await;
            if let Err(error) = result {
                let _ = cx.update(|window, cx| {
                    window.push_notification(
                        Notification::error(format!("Couldn't show {}: {error}", path.display())),
                        cx,
                    );
                });
            }
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{FileKind, entry_name_problem, name_stem};

    #[test]
    fn names_are_refused_before_sending_only_for_what_no_platform_allows() {
        let taken = |name: &str| name == "notes.md";
        for (name, refused) in [
            ("", true),
            ("a/b", true),
            ("a\\b", true),
            ("..", true),
            (".GIT", true),
            ("notes.md", true),
            ("Notes.md", false),
            (".env", false),
            ("plan.md", false),
        ] {
            assert_eq!(entry_name_problem(name, taken).is_some(), refused, "{name}");
        }
    }

    #[test]
    fn a_rename_selects_the_name_before_its_extension() {
        assert_eq!(name_stem("notes.md", FileKind::File), 0..5);
        assert_eq!(name_stem("archive.tar.gz", FileKind::File), 0..11);
        assert_eq!(name_stem(".env", FileKind::File), 0..4);
        assert_eq!(name_stem("v1.2", FileKind::Directory), 0..4);
    }
}
