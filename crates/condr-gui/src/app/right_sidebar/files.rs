//! The Files view (ADR 0018): the presented Workspace's directory tree, one level per
//! Server answer, and the menus a path affords (ADR 0045, ADR 0061).

use super::*;
use crate::app::file_icons;
use crate::app::git_marks::status_color;
use crate::app::list_row::{RowKind, row_menu, tree_indent, tree_row};
use crate::app::placeholders::{empty_state, failed_state, list_note};
use condr_core::{DirectoryEntry, FileKind};
use gpui_kit::component::scroll::ScrollableElement as _;
use std::path::{Path, PathBuf};

/// The Files tree's folds.
#[derive(Default)]
pub(in crate::app) struct FilesTreeState {
    /// Directories unfolded in the Files tree, by connection, Workspace and root-relative
    /// path.
    pub(in crate::app) expanded_dirs: HashSet<(ConnectionKey, WorkspaceId, RelativePathBuf)>,
}

/// The text "Insert Path into Terminal" writes: the relative path, single-quoted when a
/// shell would otherwise split or expand it, and a trailing space so typing can go on.
/// Single quotes are literal in POSIX shells and PowerShell alike, so `$`, backticks and
/// `$(…)` stay inert; a quote inside the name uses the POSIX `'\''` idiom.
// ponytail: cmd.exe has no single quotes; a Pane running it gets POSIX quoting.
pub(in crate::app) fn terminal_path_text(relative: &str) -> String {
    let needs_quotes = relative
        .chars()
        .any(|c| c.is_whitespace() || "\"'$&|;<>()*?[]{}!#~`\\".contains(c));
    if needs_quotes {
        format!("'{}' ", relative.replace('\'', "'\\''"))
    } else {
        format!("{relative} ")
    }
}

/// What every row of the Files tree shares.
#[derive(Clone, Copy)]
struct FilesContext<'a> {
    key: ConnectionKey,
    workspace_id: WorkspaceId,
    /// The file the Preview Tab shows, drawn selected.
    shown: Option<&'a RelativePath>,
}

/// `relative` under the Workspace root, spelled the way the root is: the root is the
/// Server's path, so a POSIX root stays `/`-separated when this client is Windows and a
/// Windows root `\`-separated when it is not.
pub(in crate::app) fn absolute_path(root: &Path, relative: &RelativePath) -> PathBuf {
    let root = root.to_string_lossy();
    PathBuf::from(if root.starts_with('/') {
        format!("{}/{relative}", root.trim_end_matches('/'))
    } else {
        format!(
            "{}\\{}",
            root.trim_end_matches(['\\', '/']),
            relative.as_str().replace('/', "\\")
        )
    })
}

/// A JetBrains file-type icon at row size, using its native palette, dimmed when ignored.
fn type_icon(path: String, ignored: bool) -> impl IntoElement {
    img(path)
        .size_4()
        .flex_shrink_0()
        .when(ignored, |this| this.opacity(0.5))
}

impl Condr {
    /// The Files view's body: the root listing unfolded as far as the user opened it.
    pub(in crate::app) fn render_files_list(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        shown: Option<&RelativePath>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = RelativePathBuf::new();
        self.ensure_directory(key, workspace_id, root.clone(), cx);
        let listing = self
            .connection(key)
            .and_then(|connection| connection.resources.directories.get(&(workspace_id, root)));
        let listing = match listing {
            None => return empty_state(t!("files.loading"), cx),
            Some(Err(reason)) => return failed_state(reason.clone(), cx),
            Some(Ok(listing)) => listing.clone(),
        };
        if listing.entries.is_empty() {
            return div()
                .id("condr-files-empty")
                .flex_1()
                .flex()
                .context_menu(self.root_context_menu(key, workspace_id, cx))
                .child(empty_state(t!("files.empty_directory"), cx))
                .into_any_element();
        }
        let context = FilesContext {
            key,
            workspace_id,
            shown,
        };
        let mut rows = Vec::new();
        self.render_directory_rows(
            &context,
            RelativePath::new(""),
            &listing.entries,
            0,
            &mut rows,
            cx,
        );
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
        // The empty area below the tree creates in the Root Directory.
        list.children(rows)
            .child(
                div()
                    .id("condr-files-root-area")
                    .debug_selector(|| "condr-files-root-area".into())
                    .flex_1()
                    .min_h(rems(2.))
                    .drag_over::<ExternalPaths>(|style, _, _, cx| style.bg(cx.theme().accent))
                    .on_drop(self.upload_drop(key, workspace_id, RelativePathBuf::new(), cx))
                    .context_menu(self.root_context_menu(key, workspace_id, cx)),
            )
            .into_any_element()
    }

    /// Asks for `path`'s listing once it is neither known nor on its way. Rendering
    /// cannot send, so the request is deferred to right after this frame.
    fn ensure_directory(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: RelativePathBuf,
        cx: &mut Context<Self>,
    ) {
        if self.connection(key).is_some_and(|connection| {
            let slot = (workspace_id, path.clone());
            !connection.resources.directories.wants(&slot)
        }) {
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
        directory: &RelativePath,
        entries: &[DirectoryEntry],
        depth: usize,
        rows: &mut Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) {
        for entry in entries {
            let path = directory.join(&entry.name);
            match entry.kind {
                FileKind::Directory => {
                    let expanded = self.right_sidebar.files.expanded_dirs.contains(&(
                        context.key,
                        context.workspace_id,
                        path.clone(),
                    ));
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
                            .resources
                            .directories
                            .get(&(context.workspace_id, path.clone()))
                    });
                    match listing {
                        None => rows.push(note_row(t!("files.loading"), depth + 1, cx)),
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
                    let selected = context.shown == Some(path.as_relative_path());
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
        path: &RelativePath,
        depth: usize,
        expanded: bool,
        ignored: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let selector: SharedString = format!("file-dir-{}", path.as_str()).into();
        let owner = cx.weak_entity();
        let menu = self.directory_context_menu(context.key, context.workspace_id, path, cx);
        let (key, workspace_id, toggle_path) = (
            context.key,
            context.workspace_id,
            path.to_relative_path_buf(),
        );
        // A folded directory still tells whether something under it changed.
        let has_changes = self
            .connection(key)
            .and_then(|connection| connection.model.workspace_git.get(&workspace_id))
            .is_some_and(|git| {
                git.change_entries()
                    .any(|entry| entry.path.starts_with(path))
            });
        tree_row(selector, depth, RowKind::Directory { open: expanded }, cx)
            .on_click(move |_, _, cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.toggle_files_directory(key, workspace_id, toggle_path.clone(), cx);
                });
            })
            .drag_over::<ExternalPaths>(|style, _, _, cx| style.bg(cx.theme().accent))
            .on_drop(self.upload_drop(key, workspace_id, path.to_relative_path_buf(), cx))
            .map(|row| row_menu(row, false, menu, cx))
            .child(type_icon(file_icons::folder_icon(theme.is_dark()), ignored))
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
                        .debug_selector(move || format!("file-dir-changed-{}", path.as_str()))
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
        path: &RelativePath,
        depth: usize,
        selected: bool,
        ignored: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let selector: SharedString = format!("file-{}", path.as_str()).into();
        let owner = cx.weak_entity();
        let menu = self.file_context_menu(context.key, context.workspace_id, path, None, cx);
        let drop = self.upload_drop(
            context.key,
            context.workspace_id,
            path.parent()
                .map(RelativePath::to_relative_path_buf)
                .unwrap_or_default(),
            cx,
        );
        let (key, workspace_id, path) = (
            context.key,
            context.workspace_id,
            path.to_relative_path_buf(),
        );
        // A changed file carries its status glyph, so the two views tell the same story.
        let status = self
            .connection(key)
            .and_then(|connection| connection.model.workspace_git.get(&workspace_id))
            .and_then(|git| git.change_entries().find(|entry| entry.path == path))
            .map(|entry| entry.status);
        tree_row(selector, depth, RowKind::File { selected }, cx)
            .on_click(move |_, window, cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.show_file_on(key, workspace_id, path.clone(), window, cx);
                });
            })
            // Dropped on a file, uploads go beside it, as New File does (ADR 0045).
            .drag_over::<ExternalPaths>(|style, _, _, cx| style.bg(cx.theme().accent))
            .on_drop(drop)
            .map(|row| row_menu(row, selected, menu, cx))
            .child(type_icon(
                file_icons::file_icon(name, theme.is_dark()),
                ignored,
            ))
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

    /// Whether the Workspace is on this machine, so its paths can be opened here.
    fn local_root(&self, key: ConnectionKey, workspace_id: WorkspaceId) -> Option<PathBuf> {
        let connection = self.connection(key)?;
        connection.endpoint.as_local_path()?;
        let root = connection
            .session()?
            .workspace(workspace_id)?
            .root_directory()
            .to_path_buf();
        root.is_absolute().then_some(root)
    }

    /// The right-click menu of a file row in either view (ADR 0018): the actions a path
    /// affords, then the edits ADR 0045 adds. Opening and revealing run on the GUI's
    /// machine, so they need a local Server like "Open in"; inserting goes to the
    /// Workspace's terminal on any Server. `show_file` is `Some(enabled)` in the Changes
    /// view, whose click opens the diff, so the menu offers the Preview Tab (ADR 0017);
    /// the Files view's click already does. Disabled there, the file is gone from disk.
    pub(in crate::app) fn file_context_menu(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: &RelativePath,
        show_file: Option<bool>,
        cx: &Context<Self>,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let owner = cx.weak_entity();
        let show_path = path.to_relative_path_buf();
        let relative = path.to_string();
        let target_pane = self
            .connection(key)
            .and_then(|connection| connection.session())
            .and_then(|session| {
                let workspace = session.workspace(workspace_id)?;
                self.insert_target_pane(key, session, workspace)
            });
        let (absolute, reveal) = self.entry_locations(key, workspace_id, path);
        let editor = self.connection(key).and_then(|connection| {
            let is_local = connection.endpoint.as_local_path().is_some();
            let workspace = connection.session()?.workspace(workspace_id)?;
            let target = self.open_target_for(open_in::project_root(workspace))?;
            let openable = absolute
                .as_ref()
                .is_some_and(|absolute| absolute.is_absolute());
            Some((
                target.id.clone(),
                target.label.clone(),
                target.icon,
                is_local && openable,
            ))
        });
        let exists = show_file != Some(false);
        // The Files view creates beside a file; Changes does not create.
        let create_in = show_file.is_none().then(|| {
            path.parent()
                .map(RelativePath::to_relative_path_buf)
                .unwrap_or_default()
        });
        move |menu, _, _| {
            let open_absolute = absolute.clone();
            let open_owner = owner.clone();
            let insert_owner = owner.clone();
            let insert_text = terminal_path_text(&relative);
            let menu = menu.when_some(show_file, |menu, enabled| {
                let show_owner = owner.clone();
                let show_path = show_path.clone();
                menu.item(
                    PopupMenuItem::new(t!("files.menu.show_file"))
                        .icon(menu_icon(Glyph::FileText))
                        .disabled(!enabled)
                        .on_click(move |_, window, cx| {
                            let _ = show_owner.update(cx, |this, cx| {
                                this.show_file_on(key, workspace_id, show_path.clone(), window, cx);
                            });
                        }),
                )
                .separator()
            });
            let menu = path_copy_items(menu, relative.clone(), absolute.clone()).separator();
            // A remote file comes here through the Stream connection (ADR 0061).
            let download_owner = owner.clone();
            let download_path = show_path.clone();
            let menu = menu
                .item(
                    PopupMenuItem::new(t!("files.menu.download"))
                        .icon(menu_icon(Glyph::Download))
                        .disabled(!exists)
                        .on_click(move |_, _, cx| {
                            let _ = download_owner.update(cx, |this, cx| {
                                this.download_file(key, workspace_id, download_path.clone(), cx)
                            });
                        }),
                )
                .separator();
            let menu = match &editor {
                Some((target_id, label, icon, enabled)) => {
                    let target_id = target_id.clone();
                    menu.item(
                        PopupMenuItem::new(t!("open_in.open_in", editor = label))
                            .icon(menu_icon(icon.icon()))
                            .disabled(!enabled || open_absolute.is_none())
                            .on_click(move |_, _, cx| {
                                let Some(absolute) = open_absolute.clone() else {
                                    return;
                                };
                                let _ = open_owner.update(cx, |this, cx| {
                                    this.open_path_in(target_id.clone(), absolute, cx);
                                });
                            }),
                    )
                }
                None => menu.item(
                    PopupMenuItem::new(t!("files.menu.open_in_editor"))
                        .icon(menu_icon(Glyph::ExternalLink))
                        .disabled(true),
                ),
            };
            let menu = menu.item(
                PopupMenuItem::new(t!("files.menu.insert_path"))
                    .icon(menu_icon(Glyph::SquareTerminal))
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
            );
            let menu = reveal_item(menu, reveal.clone(), exists, owner.clone()).separator();
            let menu = match create_in.clone() {
                Some(parent) => {
                    Condr::create_items(menu, key, workspace_id, parent, owner.clone()).separator()
                }
                None => menu,
            };
            Condr::rename_delete_items(
                menu,
                key,
                workspace_id,
                &show_path,
                FileKind::File,
                exists,
                owner.clone(),
            )
        }
    }

    /// Where a Workspace entry is: on the Server's machine, and on this one when the
    /// Workspace is local, to reveal it here.
    fn entry_locations(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: &RelativePath,
    ) -> (Option<PathBuf>, Option<PathBuf>) {
        let absolute = self
            .connection(key)
            .and_then(|connection| connection.session())
            .and_then(|session| session.workspace(workspace_id))
            .map(|workspace| absolute_path(workspace.root_directory(), path));
        let reveal = self
            .local_root(key, workspace_id)
            .map(|root| absolute_path(&root, path));
        (absolute, reveal)
    }

    /// The right-click menu of a directory row in the Files view: create inside it, the
    /// actions its path affords, and its own rename and delete.
    fn directory_context_menu(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: &RelativePath,
        cx: &Context<Self>,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let owner = cx.weak_entity();
        let (absolute, reveal) = self.entry_locations(key, workspace_id, path);
        let path = path.to_relative_path_buf();
        move |menu, _, _| {
            let menu = Condr::create_items(menu, key, workspace_id, path.clone(), owner.clone());
            let menu =
                upload_item(menu, key, workspace_id, path.clone(), owner.clone()).separator();
            let menu = path_copy_items(menu, path.to_string(), absolute.clone());
            let menu = reveal_item(menu, reveal.clone(), true, owner.clone()).separator();
            Condr::rename_delete_items(
                menu,
                key,
                workspace_id,
                &path,
                FileKind::Directory,
                true,
                owner.clone(),
            )
        }
    }

    /// The menu of the empty area below the tree: creating in the Root Directory.
    fn root_context_menu(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        cx: &Context<Self>,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        let owner = cx.weak_entity();
        move |menu, _, _| {
            let menu = Condr::create_items(
                menu,
                key,
                workspace_id,
                RelativePathBuf::new(),
                owner.clone(),
            );
            upload_item(
                menu,
                key,
                workspace_id,
                RelativePathBuf::new(),
                owner.clone(),
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
            .right_sidebar
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

    /// Opens one file in `target_id` on this machine; a failure is a toast.
    fn open_path_in(&mut self, target_id: String, path: PathBuf, cx: &mut Context<Self>) {
        let Some(target) = self
            .open_targets
            .as_deref()
            .and_then(|targets| targets.iter().find(|target| target.id == target_id))
            .cloned()
        else {
            return;
        };
        self.spawn_launch(
            t!(
                "open_in.open_failed",
                path = path.display(),
                editor = target.label
            )
            .into_owned(),
            move || open_in::launch(&target, &path),
            cx,
        )
        .detach();
    }

    /// Unfolds or folds a directory. Folding drops its listings so unfolding reads afresh.
    fn toggle_files_directory(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: RelativePathBuf,
        cx: &mut Context<Self>,
    ) {
        let dir = (key, workspace_id, path.clone());
        if self.right_sidebar.files.expanded_dirs.remove(&dir) {
            if let Some(connection) = self.connection_mut(key) {
                connection.resources.fold_directory(workspace_id, &path);
            }
        } else {
            self.right_sidebar.files.expanded_dirs.insert(dir);
            self.request_directory(key, workspace_id, path);
        }
        cx.notify();
    }

    pub(in crate::app) fn request_directory(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: RelativePathBuf,
    ) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        if let Some(request_id) = connection.send_request(|server_id, session_id, request_id| {
            ClientMessage::ListDirectory {
                server_id,
                session_id,
                request_id,
                workspace_id,
                path: path.clone(),
            }
        }) {
            connection
                .resources
                .directories
                .asked((workspace_id, path), request_id);
        }
    }
}

/// Copy Relative Path and Copy Path; the second is off when the Workspace has no root.
fn path_copy_items(menu: PopupMenu, relative: String, absolute: Option<PathBuf>) -> PopupMenu {
    let copy_absolute = absolute.clone();
    menu.item(
        PopupMenuItem::new(t!("files.menu.copy_relative_path"))
            .icon(menu_icon(Glyph::Copy))
            .on_click(move |_, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(relative.clone()));
            }),
    )
    .item(
        PopupMenuItem::new(t!("files.menu.copy_path"))
            .icon(menu_icon(Glyph::CopySlash))
            .disabled(absolute.is_none())
            .on_click(move |_, _, cx| {
                if let Some(absolute) = &copy_absolute {
                    cx.write_to_clipboard(ClipboardItem::new_string(
                        absolute.display().to_string(),
                    ));
                }
            }),
    )
}

/// "Reveal in Finder" and its counterparts, only for a Workspace on this machine.
fn reveal_item(
    menu: PopupMenu,
    absolute: Option<PathBuf>,
    enabled: bool,
    owner: WeakEntity<Condr>,
) -> PopupMenu {
    let Some(absolute) = absolute else {
        return menu;
    };
    menu.item(
        PopupMenuItem::new(reveal_label())
            .icon(menu_icon(Glyph::FolderOpen))
            .disabled(!enabled)
            .on_click(move |_, _, cx| {
                let path = absolute.clone();
                let _ = owner.update(cx, |this, cx| this.reveal_entry(path, cx));
            }),
    )
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

pub(in crate::app) fn truncated_note(cx: &App) -> AnyElement {
    list_note(
        t!("files.truncated", count = condr_core::MAX_DIRECTORY_ENTRIES),
        cx.theme().warning,
    )
    .into_any_element()
}

/// Upload Files… on a directory or the space below the tree (ADR 0061).
fn upload_item(
    menu: PopupMenu,
    key: ConnectionKey,
    workspace_id: WorkspaceId,
    directory: RelativePathBuf,
    owner: WeakEntity<Condr>,
) -> PopupMenu {
    menu.item(
        PopupMenuItem::new(t!("files.menu.upload"))
            .icon(menu_icon(Glyph::Upload))
            .on_click(move |_, window, cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.pick_uploads(key, workspace_id, directory.clone(), window, cx)
                });
            }),
    )
}

impl Condr {
    /// What a drop from the system's file manager onto `directory` does: one batch.
    fn upload_drop(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        directory: RelativePathBuf,
        cx: &Context<Self>,
    ) -> impl Fn(&ExternalPaths, &mut Window, &mut App) + 'static {
        let owner = cx.weak_entity();
        move |paths, window, cx| {
            let paths = paths.paths().to_vec();
            let _ = owner.update(cx, |this, cx| {
                this.upload_files(key, workspace_id, directory.clone(), paths, window, cx)
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{absolute_path, terminal_path_text};
    use relative_path::RelativePath;
    use std::path::Path;
    #[test]
    fn absolute_paths_follow_the_roots_own_separator() {
        assert_eq!(
            absolute_path(
                Path::new("/home/me/condr"),
                RelativePath::new("crates/core")
            )
            .to_string_lossy(),
            "/home/me/condr/crates/core"
        );
        assert_eq!(
            absolute_path(Path::new("/"), RelativePath::new("home")).to_string_lossy(),
            "/home"
        );
        assert_eq!(
            absolute_path(Path::new(r"C:\me\condr"), RelativePath::new("crates/core"))
                .to_string_lossy(),
            r"C:\me\condr\crates\core"
        );
        assert_eq!(
            absolute_path(Path::new(r"C:\"), RelativePath::new("Users")).to_string_lossy(),
            r"C:\Users"
        );
    }

    #[test]
    fn inserted_paths_are_quoted_only_when_a_shell_would_split_or_expand_them() {
        assert_eq!(terminal_path_text("src/main.rs"), "src/main.rs ");
        assert_eq!(
            terminal_path_text("docs/my notes.md"),
            "'docs/my notes.md' "
        );
        // Single quotes keep expansions inert, which double quotes would not.
        assert_eq!(terminal_path_text("a$(id).txt"), "'a$(id).txt' ");
        assert_eq!(terminal_path_text("say \"hi\".txt"), "'say \"hi\".txt' ");
        assert_eq!(terminal_path_text("it's.md"), "'it'\\''s.md' ");
    }
}
