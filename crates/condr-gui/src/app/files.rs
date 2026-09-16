//! The Files sidebar and the Preview Tab (ADR 0018): the presented Workspace's directory
//! tree, one level per Server answer, and one file's content in GPUI Kit's read-only
//! Editor. Like the Changes sidebar, everything shown is Server data.

use super::changes::{empty_state, status_color, status_glyph, tree_indent};
use super::*;
use condr_core::{DirectoryEntry, FileKind};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use std::path::Path;

/// The text "Insert Path into Terminal" writes: the relative path, quoted when a shell
/// would otherwise split or expand it, and a trailing space so typing can go on.
pub(super) fn terminal_path_text(relative: &str) -> String {
    let needs_quotes = relative
        .chars()
        .any(|c| c.is_whitespace() || "\"'$&|;<>()*?[]{}!#~`\\".contains(c));
    if needs_quotes {
        format!("\"{}\" ", relative.replace('"', "\\\""))
    } else {
        format!("{relative} ")
    }
}

/// The highlighter name for a file: its extension, which GPUI Kit's registry resolves
/// (`rs` → Rust); an unknown one leaves the text plain.
fn language_for(path: &Path) -> SharedString {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .unwrap_or_else(|| "text".to_owned())
        .into()
}

/// The Preview Tab's Editor and what it currently shows.
pub(super) struct FileEditor {
    pub(super) state: Entity<EditorState>,
    /// The path and the connection's file generation the Editor text was built from.
    shown: Option<(PathBuf, u64)>,
    pub(super) content: FileViewContent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum FileViewContent {
    Loading,
    Text,
    Binary,
    TooLarge { bytes: u64 },
    Failed(String),
}

/// What every row of the Files tree shares.
#[derive(Clone, Copy)]
struct FilesContext<'a> {
    key: ConnectionKey,
    workspace_id: WorkspaceId,
    /// The file the Preview Tab shows, drawn selected.
    shown: Option<&'a Path>,
}

fn path_text(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}

/// A file-type icon at row size, dimmed when ignored. The Material set is drawn for
/// VS Code's brighter chrome and glared on Condr's dark panels; the fix is the theme's own
/// "saturation" setting, baked into the SVGs by `script/generate-file-icons.mjs`. Lowering
/// opacity as well was tried and made the icons look washed out.
fn type_icon(path: String, ignored: bool) -> impl IntoElement {
    img(path)
        .size_4()
        .flex_shrink_0()
        .when(ignored, |this| this.opacity(0.5))
}

impl Condr {
    /// The Files view's body: the root listing unfolded as far as the user opened it.
    pub(super) fn render_files_list(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        shown: Option<&Path>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = PathBuf::new();
        self.ensure_directory(key, workspace_id, root.clone(), cx);
        let listing = self
            .connection(key)
            .and_then(|connection| connection.directories.get(&(workspace_id, root)));
        let listing = match listing {
            None => return empty_state("Loading…", cx),
            Some(Err(reason)) => return failed_state(reason.clone(), cx),
            Some(Ok(listing)) => listing.clone(),
        };
        if listing.entries.is_empty() {
            return empty_state("Empty directory", cx);
        }
        let context = FilesContext {
            key,
            workspace_id,
            shown,
        };
        let mut rows = Vec::new();
        self.render_directory_rows(&context, Path::new(""), &listing.entries, 0, &mut rows, cx);
        let mut list = v_flex()
            .id("condr-files-list")
            .debug_selector(|| "condr-files-list".into())
            .flex_1()
            .min_h_0()
            .w_full()
            .px_1()
            .pb_1()
            .overflow_y_scrollbar();
        if listing.truncated {
            list = list.child(truncated_note(cx));
        }
        list.children(rows).into_any_element()
    }

    /// Asks for `path`'s listing once it is neither known nor on its way. Rendering
    /// cannot send, so the request is deferred to right after this frame.
    fn ensure_directory(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let known = self.connection(key).is_some_and(|connection| {
            connection
                .directories
                .contains_key(&(workspace_id, path.clone()))
        });
        if known
            || self
                .pending_directories
                .contains(&(key, workspace_id, path.clone()))
        {
            return;
        }
        let owner = cx.weak_entity();
        cx.defer(move |cx| {
            let _ = owner.update(cx, |this, _| {
                this.request_directory(key, workspace_id, path)
            });
        });
    }

    fn render_directory_rows(
        &self,
        context: &FilesContext<'_>,
        directory: &Path,
        entries: &[DirectoryEntry],
        depth: usize,
        rows: &mut Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) {
        for entry in entries {
            let path = directory.join(&entry.name);
            match entry.kind {
                FileKind::Directory => {
                    let expanded = self
                        .expanded_dirs
                        .contains(&(context.workspace_id, path.clone()));
                    rows.push(self.render_directory_row(
                        context,
                        &entry.name,
                        &path,
                        depth,
                        expanded,
                        entry.ignored,
                        cx,
                    ));
                    if !expanded {
                        continue;
                    }
                    self.ensure_directory(context.key, context.workspace_id, path.clone(), cx);
                    let listing = self.connection(context.key).and_then(|connection| {
                        connection
                            .directories
                            .get(&(context.workspace_id, path.clone()))
                    });
                    match listing {
                        None => rows.push(note_row("Loading…", depth + 1, cx)),
                        Some(Err(reason)) => rows.push(note_row(reason.clone(), depth + 1, cx)),
                        Some(Ok(listing)) => {
                            if listing.truncated {
                                rows.push(truncated_note(cx));
                            }
                            let children = listing.entries.clone();
                            self.render_directory_rows(
                                context,
                                &path,
                                &children,
                                depth + 1,
                                rows,
                                cx,
                            );
                        }
                    }
                }
                FileKind::File => {
                    let selected = context.shown == Some(path.as_path());
                    rows.push(self.render_file_row(
                        context,
                        &entry.name,
                        &path,
                        depth,
                        selected,
                        entry.ignored,
                        cx,
                    ));
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_directory_row(
        &self,
        context: &FilesContext<'_>,
        name: &str,
        path: &Path,
        depth: usize,
        expanded: bool,
        ignored: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let selector: SharedString = format!("file-dir-{}", path_text(path)).into();
        let id = selector.clone();
        let owner = cx.weak_entity();
        let (key, workspace_id, toggle_path) =
            (context.key, context.workspace_id, path.to_path_buf());
        // A folded directory still tells whether something under it changed.
        let has_changes = self
            .connection(key)
            .and_then(|connection| connection.workspace_git.get(&workspace_id))
            .is_some_and(|git| {
                git.changes
                    .entries
                    .iter()
                    .any(|entry| entry.path.starts_with(path))
            });
        h_flex()
            .id(id)
            .debug_selector(move || selector.to_string())
            .h_7()
            .w_full()
            .min_w_0()
            .pl(tree_indent(depth, false))
            .pr_2()
            .gap_1()
            .items_center()
            .cursor_pointer()
            .rounded(theme.radius)
            .text_sm()
            .hover(|this| this.bg(theme.sidebar_accent.opacity(0.8)))
            .on_click(move |_, _, cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.toggle_files_directory(key, workspace_id, toggle_path.clone(), cx);
                });
            })
            .child(
                Icon::new(IconName::ChevronRight)
                    .size_3p5()
                    .text_color(theme.muted_foreground)
                    .when(expanded, |icon| icon.rotate(percentage(90. / 360.))),
            )
            .child(type_icon(file_icons::folder_icon(name), ignored))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .when(ignored, |this| this.text_color(theme.muted_foreground))
                    .child(name.to_owned()),
            )
            .when(has_changes, |this| {
                this.child(
                    div()
                        .debug_selector(move || format!("file-dir-changed-{}", path_text(path)))
                        .flex_none()
                        .size(px(6.))
                        .rounded_full()
                        .bg(theme.warning),
                )
            })
            .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_file_row(
        &self,
        context: &FilesContext<'_>,
        name: &str,
        path: &Path,
        depth: usize,
        selected: bool,
        ignored: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let selector: SharedString = format!("file-{}", path_text(path)).into();
        let id = selector.clone();
        let owner = cx.weak_entity();
        let menu = self.file_context_menu(context.key, context.workspace_id, path, cx);
        let (key, workspace_id, path) = (context.key, context.workspace_id, path.to_path_buf());
        // A changed file carries its status glyph, so the two views tell the same story.
        let status = self
            .connection(key)
            .and_then(|connection| connection.workspace_git.get(&workspace_id))
            .and_then(|git| git.changes.entries.iter().find(|entry| entry.path == path))
            .map(|entry| entry.status);
        h_flex()
            .id(id)
            .debug_selector(move || selector.to_string())
            .h_7()
            .w_full()
            .min_w_0()
            .pl(tree_indent(depth, true))
            .pr_2()
            .gap_2()
            .items_center()
            .cursor_pointer()
            .rounded(theme.radius)
            .text_sm()
            .when(!selected, |this| {
                this.hover(|this| {
                    this.bg(theme.sidebar_accent.opacity(0.8))
                        .text_color(theme.sidebar_accent_foreground)
                })
            })
            .when(selected, |this| {
                this.bg(theme.tokens.sidebar_accent)
                    .text_color(theme.sidebar_accent_foreground)
            })
            .on_click(move |_, window, cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.show_file_on(key, workspace_id, path.clone(), window, cx);
                });
            })
            .context_menu(menu)
            .child(type_icon(file_icons::file_icon(name), ignored))
            // The type icon keeps the slot; a change shows in the name's colour, as
            // VS Code paints it, and in the Changes view's glyph.
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .when(ignored && !selected, |this| {
                        this.text_color(theme.muted_foreground)
                    })
                    .when_some(status.filter(|_| !selected), |this, status| {
                        this.text_color(status_color(status, cx))
                    })
                    .child(name.to_owned()),
            )
            .into_any_element()
    }

    /// The right-click menu of a file row in either view (ADR 0018): the read-only
    /// actions a path affords. Opening runs on the GUI's machine, so it needs a local
    /// Server like "Open in"; inserting goes to the Workspace's terminal on any Server.
    pub(super) fn file_context_menu(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: &Path,
        cx: &Context<Self>,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let owner = cx.weak_entity();
        let relative = path_text(path);
        let (root, target_pane) = self
            .connection(key)
            .and_then(|connection| Session::restore(connection.snapshot.clone()).ok())
            .and_then(|session| {
                let workspace = session.workspace(workspace_id)?;
                let root = workspace.root_directory().to_path_buf();
                let target_pane = self.insert_target_pane(key, &session, workspace);
                Some((Some(root), target_pane))
            })
            .unwrap_or((None, None));
        let editor = self.connection(key).and_then(|connection| {
            let is_local = connection.endpoint.as_local_path().is_some();
            let root = root.as_deref()?;
            let session = Session::restore(connection.snapshot.clone()).ok()?;
            let workspace = session.workspace(workspace_id)?;
            let target = self.open_target_for(open_in::project_root(workspace))?;
            Some((
                target.id.clone(),
                target.label.clone(),
                is_local && root.is_absolute(),
            ))
        });
        let absolute = root.map(|root| root.join(path));
        move |menu, _, _| {
            let copy_relative = relative.clone();
            let copy_absolute = absolute.clone();
            let open_absolute = absolute.clone();
            let open_owner = owner.clone();
            let insert_owner = owner.clone();
            let insert_text = terminal_path_text(&relative);
            let menu = menu
                .item(
                    PopupMenuItem::new("Copy Relative Path").on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy_relative.clone()));
                    }),
                )
                .item(
                    PopupMenuItem::new("Copy Path")
                        .disabled(copy_absolute.is_none())
                        .on_click(move |_, _, cx| {
                            if let Some(absolute) = &copy_absolute {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    absolute.display().to_string(),
                                ));
                            }
                        }),
                )
                .separator();
            let menu = match &editor {
                Some((target_id, label, enabled)) => {
                    let target_id = target_id.clone();
                    menu.item(
                        PopupMenuItem::new(format!("Open in {label}"))
                            .disabled(!enabled || open_absolute.is_none())
                            .on_click(move |_, window, cx| {
                                let Some(absolute) = open_absolute.clone() else {
                                    return;
                                };
                                let _ = open_owner.update(cx, |this, cx| {
                                    this.open_path_in(target_id.clone(), absolute, window, cx);
                                });
                            }),
                    )
                }
                None => menu.item(PopupMenuItem::new("Open in Editor").disabled(true)),
            };
            menu.item(
                PopupMenuItem::new("Insert Path into Terminal")
                    .disabled(target_pane.is_none())
                    .on_click(move |_, window, cx| {
                        let Some((tab_id, pane_id)) = target_pane else {
                            return;
                        };
                        let _ = insert_owner.update(cx, |this, cx| {
                            this.terminal_command(
                                key,
                                pane_id,
                                TerminalCommand::Paste(insert_text.clone()),
                            );
                            this.activate_tab_on(key, tab_id, window, cx);
                        });
                    }),
            )
        }
    }

    /// The terminal "Insert Path" writes to: the Workspace's last presented terminal Tab,
    /// else its first one; `None` when it has no terminal Tab at all.
    fn insert_target_pane(
        &self,
        key: ConnectionKey,
        session: &Session,
        workspace: &condr_core::Workspace,
    ) -> Option<(TabId, PaneId)> {
        let tab = self
            .last_terminal_tabs
            .get(&(key, workspace.id()))
            .and_then(|tab_id| workspace.tabs().iter().find(|tab| tab.id() == *tab_id))
            .filter(|tab| tab.terminals().is_some())
            .or_else(|| {
                workspace
                    .tabs()
                    .iter()
                    .find(|tab| tab.terminals().is_some())
            })?;
        let pane = session.tab(tab.id())?.focused_pane()?;
        Some((tab.id(), pane.id()))
    }

    /// Opens one file in `target_id` on this machine; failure shows as a notification.
    fn open_path_in(
        &mut self,
        target_id: String,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self
            .open_targets
            .as_deref()
            .and_then(|targets| targets.iter().find(|target| target.id == target_id))
            .cloned()
        else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_spawn({
                    let (target, path) = (target.clone(), path.clone());
                    async move { open_in::launch(&target, &path) }
                })
                .await;
            if let Err(error) = result {
                let _ = this.update_in(cx, |_, window, cx| {
                    window.push_notification(
                        Notification::error(format!(
                            "Couldn't open {} in {}: {error}",
                            path.display(),
                            target.label
                        )),
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    /// Unfolds or folds a directory. Folding drops its listings so unfolding reads afresh.
    fn toggle_files_directory(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: PathBuf,
        cx: &mut Context<Self>,
    ) {
        let dir = (workspace_id, path.clone());
        if self.expanded_dirs.remove(&dir) {
            if let Some(connection) = self.connection_mut(key) {
                connection
                    .directories
                    .retain(|(listed_workspace, listed), _| {
                        *listed_workspace != workspace_id || !listed.starts_with(&path)
                    });
            }
            self.pending_directories
                .retain(|(pending_key, pending_workspace, pending)| {
                    *pending_key != key
                        || *pending_workspace != workspace_id
                        || !pending.starts_with(&path)
                });
        } else {
            self.expanded_dirs.insert(dir);
            self.request_directory(key, workspace_id, path);
        }
        cx.notify();
    }

    /// Shows `path` in the Workspace's Preview Tab: a layout command like any other,
    /// presented once the Server confirms it.
    fn show_file_on(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_presenting_layout_to(
            key,
            LayoutCommand::ShowFile { workspace_id, path },
            window,
            cx,
        );
    }

    /// The Preview Tab's body: one file's content, or why there is none.
    pub(super) fn render_file_tab(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: TabId,
        path: &Path,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let path_text = path_text(path);
        let entry = self
            .connection(key)
            .and_then(|connection| connection.workspace_git.get(&workspace_id))
            .and_then(|git| git.changes.entries.iter().find(|entry| entry.path == path));
        let editor = self.file_editors.get(&(key, tab_id));
        let content = editor.map_or(FileViewContent::Loading, |editor| editor.content.clone());
        let header = h_flex()
            .debug_selector(|| "file-header".into())
            .flex_none()
            .h_9()
            .w_full()
            .px_3()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(type_icon(file_icons::file_icon_for_path(path), false))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_sm()
                    .font_medium()
                    .child(path_text.clone()),
            )
            .when_some(entry, |this, entry| {
                this.child(status_glyph(entry.status, cx))
            });
        let body = match (&content, editor) {
            (FileViewContent::Text, Some(editor)) => div()
                .debug_selector(|| "file-body".into())
                .flex_1()
                .min_h_0()
                .w_full()
                .child(
                    Editor::new(&editor.state)
                        .readonly(true)
                        .appearance(false)
                        .bordered(false)
                        .font_family(self.terminal_font.family.clone())
                        .text_size(px(self.terminal_font.size))
                        .h(relative(1.)),
                )
                .into_any_element(),
            (FileViewContent::Loading, _) | (FileViewContent::Text, None) => {
                empty_state("Loading file…", cx)
            }
            (FileViewContent::Binary, _) => empty_state("Binary file", cx),
            (FileViewContent::TooLarge { bytes }, _) => empty_state(
                format!(
                    "File too large to show ({:.1} MiB)",
                    *bytes as f64 / (1024. * 1024.)
                ),
                cx,
            ),
            (FileViewContent::Failed(reason), _) => failed_state(reason.clone(), cx),
        };
        v_flex()
            .debug_selector(|| "file-tab".into())
            .size_full()
            .child(header)
            .child(body)
            .into_any_element()
    }

    /// Brings the Preview Tab's Editor in line with the Server's answer for its file.
    /// Called from `rebuild_dock`, the one place that sees every presentation change.
    pub(super) fn sync_file_view(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: TabId,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.connection(key) else {
            return;
        };
        let generation = connection.files_generation;
        let answer = connection.files.get(&(workspace_id, path.clone())).cloned();
        if answer.is_none()
            && !self
                .pending_files
                .contains(&(key, workspace_id, path.clone()))
        {
            self.request_file(key, workspace_id, path.clone());
        }

        let editor = self.file_editors.entry((key, tab_id)).or_insert_with(|| {
            let state = cx.new(|cx| {
                EditorState::new(window, cx)
                    // Any language puts the Editor in code mode; the real one is set per
                    // file below, since one Tab shows many files over its life.
                    .language("text")
                    .line_number(true)
                    .soft_wrap(false)
                    .indent_guides(false)
                    .folding(false)
                    .searchable(true)
            });
            FileEditor {
                state,
                shown: None,
                content: FileViewContent::Loading,
            }
        });
        let Some(answer) = answer else {
            if editor
                .shown
                .as_ref()
                .is_none_or(|(shown, _)| *shown != path)
            {
                editor.content = FileViewContent::Loading;
                editor.shown = None;
            }
            return;
        };
        if editor.shown.as_ref() == Some(&(path.clone(), generation)) {
            return;
        }
        let shown_path = path.clone();
        editor.shown = Some((path, generation));
        let (text, content) = match answer {
            Err(reason) => (String::new(), FileViewContent::Failed(reason)),
            Ok(FileContent::Binary) => (String::new(), FileViewContent::Binary),
            Ok(FileContent::TooLarge { bytes }) => {
                (String::new(), FileViewContent::TooLarge { bytes })
            }
            Ok(FileContent::Text { text }) => (text, FileViewContent::Text),
        };
        editor.content = content;
        let language = language_for(&shown_path);
        editor.state.update(cx, |state, cx| {
            state.set_highlighter(language, cx);
            state.set_value(text, window, cx);
        });
    }

    pub(super) fn request_directory(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: PathBuf,
    ) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let (Some(server_id), Some(session_id)) = (connection.server_id, connection.session_id)
        else {
            return;
        };
        if connection.status != ConnectionStatus::Connected {
            return;
        }
        let request_id = connection.next_layout_request_id;
        connection.next_layout_request_id = request_id.wrapping_add(1).max(1);
        connection.send(ClientMessage::ListDirectory {
            server_id,
            session_id,
            request_id,
            workspace_id,
            path: path.clone(),
        });
        self.pending_directories.insert((key, workspace_id, path));
    }

    fn request_file(&mut self, key: ConnectionKey, workspace_id: WorkspaceId, path: PathBuf) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let (Some(server_id), Some(session_id)) = (connection.server_id, connection.session_id)
        else {
            return;
        };
        if connection.status != ConnectionStatus::Connected {
            return;
        }
        let request_id = connection.next_layout_request_id;
        connection.next_layout_request_id = request_id.wrapping_add(1).max(1);
        connection.send(ClientMessage::ReadFile {
            server_id,
            session_id,
            request_id,
            workspace_id,
            path: path.clone(),
        });
        self.pending_files.insert((key, workspace_id, path));
    }

    /// The working tree moved: asks again for every listing and file of the Workspace
    /// that is cached, keeping the old answers on screen until the new ones land.
    pub(super) fn refresh_workspace_files(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
    ) {
        let Some(connection) = self.connection(key) else {
            return;
        };
        let directories: Vec<PathBuf> = connection
            .directories
            .keys()
            .filter(|(listed_workspace, _)| *listed_workspace == workspace_id)
            .map(|(_, path)| path.clone())
            .collect();
        let files: Vec<PathBuf> = connection
            .files
            .keys()
            .filter(|(read_workspace, _)| *read_workspace == workspace_id)
            .map(|(_, path)| path.clone())
            .collect();
        self.pending_directories
            .retain(|(pending_key, pending_workspace, _)| {
                *pending_key != key || *pending_workspace != workspace_id
            });
        self.pending_files
            .retain(|(pending_key, pending_workspace, _)| {
                *pending_key != key || *pending_workspace != workspace_id
            });
        for path in directories {
            self.request_directory(key, workspace_id, path);
        }
        for path in files {
            self.request_file(key, workspace_id, path);
        }
    }

    /// Drops Editors whose Tabs are gone, so a closed Preview Tab frees its text.
    pub(super) fn prune_file_editors(&mut self) {
        let sessions = self.restored_sessions();
        self.file_editors.retain(|(key, tab_id), _| {
            sessions
                .get(key)
                .is_some_and(|session| session.tab(*tab_id).is_some_and(|tab| tab.file().is_some()))
        });
    }
}

fn failed_state(reason: String, cx: &App) -> AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .p_4()
        .text_sm()
        .text_color(cx.theme().danger)
        .child(reason)
        .into_any_element()
}

/// A muted note under a directory row: its listing loading, or why it failed.
fn note_row(text: impl Into<SharedString>, depth: usize, cx: &App) -> AnyElement {
    div()
        .h_7()
        .w_full()
        .pl(tree_indent(depth, true))
        .pr_2()
        .flex()
        .items_center()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .truncate()
        .child(text.into())
        .into_any_element()
}

fn truncated_note(cx: &App) -> AnyElement {
    div()
        .px_2()
        .py_1()
        .text_xs()
        .text_color(cx.theme().warning)
        .child(format!(
            "Only the first {} entries are listed.",
            condr_core::MAX_DIRECTORY_ENTRIES
        ))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{language_for, terminal_path_text};
    use std::path::Path;

    #[test]
    fn inserted_paths_are_quoted_only_when_a_shell_would_split_them() {
        assert_eq!(terminal_path_text("src/main.rs"), "src/main.rs ");
        assert_eq!(
            terminal_path_text("docs/my notes.md"),
            "\"docs/my notes.md\" "
        );
        assert_eq!(terminal_path_text("a$b"), "\"a$b\" ");
        assert_eq!(
            terminal_path_text("say \"hi\".txt"),
            "\"say \\\"hi\\\".txt\" "
        );
    }

    #[test]
    fn the_highlighter_follows_the_extension_and_falls_back_to_plain_text() {
        assert_eq!(language_for(Path::new("src/main.RS")).as_ref(), "rs");
        assert_eq!(language_for(Path::new("Cargo.toml")).as_ref(), "toml");
        assert_eq!(language_for(Path::new("Dockerfile")).as_ref(), "text");
        assert_eq!(language_for(Path::new(".gitignore")).as_ref(), "text");
    }
}
