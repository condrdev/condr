//! The Files sidebar and the Preview Tab (ADR 0018): the presented Workspace's directory
//! tree, one level per Server answer, and one file's content in GPUI Kit's read-only
//! Editor, or rendered when it is Markdown or an image (ADR 0037). Like the Changes
//! sidebar, everything shown is Server data.

use super::changes::{empty_state, status_color, status_glyph, tree_indent};
use super::preview::{
    DecodedImage, PreviewImages, Rendering, image_summary, release_images, render_image,
    rendering_for,
};
use super::*;
use condr_core::{DirectoryEntry, FileKind};
use gpui_kit::base::text::TextViewState;
use gpui_kit::component::button::ButtonGroup;
use gpui_kit::component::input::{Position, RopeExt as _};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use std::path::{Path, PathBuf};

/// Local Files/Changes presentation, retained against the authoritative structure.
#[derive(Default)]
pub(super) struct FilesViewState {
    /// Workspaces showing the right sidebar (ADR 0017); remembered in the GUI state
    /// file alongside `sidebar_workspace_open` (ADR 0023).
    pub(super) changes_open: HashSet<(ConnectionKey, WorkspaceId)>,
    /// The Workspaces whose Changes and Diff Tab compare against the base (ADR 0034); the
    /// others compare against `HEAD`. This Client's view, remembered in the state file.
    pub(super) changes_against_base: HashSet<(ConnectionKey, WorkspaceId)>,
    pub(super) collapsed_changes_sections: HashSet<ChangesSection>,
    /// Directories folded shut in the Changes tree, by connection, Workspace and
    /// repository path; Workspace ids repeat across Servers.
    pub(super) collapsed_change_dirs: HashSet<(ConnectionKey, WorkspaceId, RelativePathBuf)>,
    /// The Diff Tabs' Editors, by connection and Tab; pruned with the Tabs.
    pub(super) diff_editors: HashMap<(ConnectionKey, TabId), DiffEditor>,
    /// Which view the right sidebar shows per Workspace (ADR 0018), remembered in the
    /// GUI state file. Others open on Changes, or Files outside a repository.
    pub(super) sidebar_views: HashMap<(ConnectionKey, WorkspaceId), SidebarView>,
    /// The terminal Tab each Workspace presented last: where "Insert Path into Terminal"
    /// sends its text once a viewer Tab has taken the Workspace's active slot.
    pub(super) last_terminal_tabs: HashMap<(ConnectionKey, WorkspaceId), TabId>,
    /// Directories unfolded in the Files tree, by connection, Workspace and root-relative
    /// path.
    pub(super) expanded_dirs: HashSet<(ConnectionKey, WorkspaceId, RelativePathBuf)>,
    /// The Preview Tabs' Editors, by connection and Tab; pruned with the Tabs.
    pub(super) file_editors: HashMap<(ConnectionKey, TabId), FileEditor>,
    /// The 0-based line the Preview Tab lands on once it shows this file: set by the Diff
    /// Tab's "Show File", applied by `sync_file_view` when the text is there, then cleared.
    /// Presentation only, so it never rides `ShowFile`.
    pub(super) pending_file_line: Option<(ConnectionKey, WorkspaceId, RelativePathBuf, u32)>,
    /// The images the Preview Tabs show, decoded (ADR 0037).
    pub(super) images: PreviewImages,
}

impl FilesViewState {
    /// Hands back the images it dropped, for `release_images` to free from the atlas.
    pub(super) fn retain_session(
        &mut self,
        key: ConnectionKey,
        session: Option<&Session>,
    ) -> Vec<Arc<DecodedImage>> {
        let workspace_live = |connection_key: ConnectionKey, id: WorkspaceId| {
            connection_key != key || session.is_some_and(|session| session.workspace(id).is_some())
        };
        self.changes_open
            .retain(|(connection, id)| workspace_live(*connection, *id));
        self.changes_against_base
            .retain(|(connection, id)| workspace_live(*connection, *id));
        self.expanded_dirs
            .retain(|(connection, id, _)| workspace_live(*connection, *id));
        self.collapsed_change_dirs
            .retain(|(connection, id, _)| workspace_live(*connection, *id));
        self.sidebar_views
            .retain(|(connection, id), _| workspace_live(*connection, *id));
        self.last_terminal_tabs.retain(|(connection, id), tab_id| {
            workspace_live(*connection, *id)
                && (*connection != key
                    || session.is_some_and(|session| {
                        session
                            .workspace(*id)
                            .and_then(|workspace| workspace.tab(*tab_id))
                            .is_some_and(|tab| tab.terminals().is_some())
                    }))
        });
        self.diff_editors.retain(|(connection, id), _| {
            *connection != key
                || session
                    .is_some_and(|session| session.tab(*id).is_some_and(|tab| tab.diff().is_some()))
        });
        self.file_editors.retain(|(connection, id), _| {
            *connection != key
                || session
                    .is_some_and(|session| session.tab(*id).is_some_and(|tab| tab.file().is_some()))
        });
        if self
            .pending_file_line
            .as_ref()
            .is_some_and(|(connection, id, _, _)| !workspace_live(*connection, *id))
        {
            self.pending_file_line = None;
        }
        // A Workspace's images are its Preview Tab's, so they go when that Tab closes.
        self.images.retain(|(connection, id, _)| {
            *connection != key
                || session
                    .and_then(|session| session.workspace(*id))
                    .is_some_and(|workspace| {
                        workspace.tabs().iter().any(|tab| tab.file().is_some())
                    })
        })
    }
}

/// The byte range of `old` that differs from `new`, and the text of `new` that replaces it.
fn changed_span<'a>(old: &str, new: &'a str) -> (std::ops::Range<usize>, &'a str) {
    let mut prefix = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(prefix) || !new.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let mut suffix = old
        .bytes()
        .rev()
        .zip(new.bytes().rev())
        .take(old.len().min(new.len()) - prefix)
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
        suffix -= 1;
    }
    (prefix..old.len() - suffix, &new[prefix..new.len() - suffix])
}

/// The text "Insert Path into Terminal" writes: the relative path, single-quoted when a
/// shell would otherwise split or expand it, and a trailing space so typing can go on.
/// Single quotes are literal in POSIX shells and PowerShell alike, so `$`, backticks and
/// `$(…)` stay inert; a quote inside the name uses the POSIX `'\''` idiom.
// ponytail: cmd.exe has no single quotes; a Pane running it gets POSIX quoting.
pub(super) fn terminal_path_text(relative: &str) -> String {
    let needs_quotes = relative
        .chars()
        .any(|c| c.is_whitespace() || "\"'$&|;<>()*?[]{}!#~`\\".contains(c));
    if needs_quotes {
        format!("'{}' ", relative.replace('\'', "'\\''"))
    } else {
        format!("{relative} ")
    }
}

/// The highlighter name for a file: its extension, which `syntax::highlighter_factory` resolves
/// (`rs` → Rust); an unknown one leaves the text plain.
fn language_for(path: &RelativePath) -> SharedString {
    path.extension()
        .map(|extension| extension.to_ascii_lowercase())
        .unwrap_or_else(|| "text".to_owned())
        .into()
}

/// Takes the line the Diff Tab asked the Preview Tab to land on, if it is for this file.
fn take_pending_line(
    pending: &mut Option<(ConnectionKey, WorkspaceId, RelativePathBuf, u32)>,
    key: ConnectionKey,
    workspace_id: WorkspaceId,
    path: &RelativePath,
) -> Option<u32> {
    let (pending_key, pending_workspace, pending_path, _) = pending.as_ref()?;
    if *pending_key != key || *pending_workspace != workspace_id || pending_path != path {
        return None;
    }
    pending.take().map(|(_, _, _, line)| line)
}

/// Puts the cursor at the start of the 0-based `line`, clamped to the text; Kit scrolls
/// the cursor into view and focuses the Editor.
fn land_on_line(
    state: &mut EditorState,
    line: u32,
    window: &mut Window,
    cx: &mut Context<EditorState>,
) {
    let last = state.text().lines_len().saturating_sub(1) as u32;
    state.set_cursor_position(Position::new(line.min(last), 0), window, cx);
}

/// The Preview Tab's Editor and what it currently shows.
pub(super) struct FileEditor {
    pub(super) state: Entity<EditorState>,
    /// The path and the generation of the cached answer the Editor text was built from.
    shown: Option<(RelativePathBuf, u64)>,
    pub(super) content: FileViewContent,
    /// The file rendered, while it is Markdown (ADR 0037).
    pub(super) markdown: Option<Entity<TextViewState>>,
    /// Where the rendered file was read, and its block count then, while an edit to it
    /// may still reset Kit's list to the top; see `render_markdown`.
    pub(super) markdown_scroll: Rc<Cell<Option<(usize, ListOffset)>>>,
    /// A file the Tab can render shows its text instead. Each file opens rendered, except
    /// one "Show File" opens at a line.
    pub(super) source: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum FileViewContent {
    Loading,
    Text,
    /// Decoded into `FilesViewState::images`.
    Image,
    Binary,
    TooLarge {
        bytes: u64,
    },
    Failed(String),
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
pub(super) fn absolute_path(root: &Path, relative: &RelativePath) -> PathBuf {
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
    pub(super) fn render_files_list(
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
        list.children(rows).into_any_element()
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
            connection.resources.directories.contains_key(&slot)
                || connection.resources.pending_directories.contains_key(&slot)
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
                    let expanded = self.files_view.expanded_dirs.contains(&(
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
        let id = selector.clone();
        let owner = cx.weak_entity();
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
        let id = selector.clone();
        let owner = cx.weak_entity();
        let menu = self.file_context_menu(context.key, context.workspace_id, path, None, cx);
        let (key, workspace_id, path) = (
            context.key,
            context.workspace_id,
            path.to_relative_path_buf(),
        );
        // A changed file carries its status glyph, so the two views tell the same story.
        let status = self
            .connection(key)
            .and_then(|connection| connection.model.workspace_git.get(&workspace_id))
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

    /// The right-click menu of a file row in either view (ADR 0018): the read-only
    /// actions a path affords. Opening runs on the GUI's machine, so it needs a local
    /// Server like "Open in"; inserting goes to the Workspace's terminal on any Server.
    /// `show_file` is `Some(enabled)` in the Changes view, whose click opens the diff, so
    /// the menu offers the Preview Tab (ADR 0017); the Files view's click already does.
    pub(super) fn file_context_menu(
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
        let (root, target_pane) = self
            .connection(key)
            .and_then(|connection| connection.session())
            .and_then(|session| {
                let workspace = session.workspace(workspace_id)?;
                let root = workspace.root_directory().to_path_buf();
                let target_pane = self.insert_target_pane(key, session, workspace);
                Some((Some(root), target_pane))
            })
            .unwrap_or((None, None));
        let editor = self.connection(key).and_then(|connection| {
            let is_local = connection.endpoint.as_local_path().is_some();
            let root = root.as_deref()?;
            let session = connection.session()?;
            let workspace = session.workspace(workspace_id)?;
            let target = self.open_target_for(open_in::project_root(workspace))?;
            Some((
                target.id.clone(),
                target.label.clone(),
                is_local && root.is_absolute(),
            ))
        });
        let absolute = root.map(|root| absolute_path(&root, path));
        move |menu, _, _| {
            let copy_relative = relative.clone();
            let copy_absolute = absolute.clone();
            let open_absolute = absolute.clone();
            let open_owner = owner.clone();
            let insert_owner = owner.clone();
            let insert_text = terminal_path_text(&relative);
            let menu = menu.when_some(show_file, |menu, enabled| {
                let show_owner = owner.clone();
                let show_path = show_path.clone();
                menu.item(PopupMenuItem::new("Show File").disabled(!enabled).on_click(
                    move |_, window, cx| {
                        let _ = show_owner.update(cx, |this, cx| {
                            this.show_file_on(key, workspace_id, show_path.clone(), window, cx);
                        });
                    },
                ))
                .separator()
            });
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
            .files_view
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
        cx.spawn_in(window, async move |_, cx| {
            let result = cx
                .background_spawn({
                    let (target, path) = (target.clone(), path.clone());
                    async move { open_in::launch(&target, &path) }
                })
                .await;
            if let Err(error) = result {
                let _ = cx.update(|window, cx| {
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
        path: RelativePathBuf,
        cx: &mut Context<Self>,
    ) {
        let dir = (key, workspace_id, path.clone());
        if self.files_view.expanded_dirs.remove(&dir) {
            if let Some(connection) = self.connection_mut(key) {
                connection.resources.fold_directory(workspace_id, &path);
            }
        } else {
            self.files_view.expanded_dirs.insert(dir);
            self.request_directory(key, workspace_id, path);
        }
        cx.notify();
    }

    /// Shows `path` in the Workspace's Preview Tab: a layout command like any other,
    /// presented once the Server confirms it.
    pub(super) fn show_file_on(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: RelativePathBuf,
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

    /// The Preview Tab's body: one file's content, rendered when it can be, or why there
    /// is none.
    pub(super) fn render_file_tab(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: TabId,
        path: &RelativePath,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let editor = self.files_view.file_editors.get(&(key, tab_id));
        let content = editor.map_or(FileViewContent::Loading, |editor| editor.content.clone());
        let rendering = rendering_for(path).filter(|_| content == FileViewContent::Text);
        let source = editor.is_some_and(|editor| editor.source);
        let image = (content == FileViewContent::Image
            || (rendering == Some(Rendering::Svg) && !source))
            .then(|| {
                self.files_view
                    .images
                    .get(&(key, workspace_id, path.to_relative_path_buf()))
            });
        let header = self.render_file_header(
            key,
            workspace_id,
            tab_id,
            path,
            rendering,
            source,
            image.as_ref(),
            cx,
        );
        let markdown = editor
            .and_then(|editor| Some((editor.markdown.clone()?, editor.markdown_scroll.clone())))
            .filter(|_| rendering == Some(Rendering::Markdown) && !source);
        let body = match (image, markdown, &content, editor) {
            (Some(None), ..) => empty_state("Loading image…", cx),
            (Some(Some(Ok(image))), ..) => render_image(image),
            (Some(Some(Err(reason))), ..) => empty_state(reason, cx),
            (None, Some((markdown, scroll)), ..) => {
                self.render_markdown(key, workspace_id, path, &markdown, scroll, cx)
            }
            (None, None, FileViewContent::Text, Some(editor)) => {
                let theme = cx.theme();
                div()
                    .debug_selector(|| "file-body".into())
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    // The code theme's background: the Editor draws none of its own.
                    .when_some(
                        theme.highlight_theme.style.editor_background,
                        |this, background| this.bg(background),
                    )
                    .child(
                        Editor::new(&editor.state)
                            .readonly(true)
                            .appearance(false)
                            .bordered(false)
                            .font_family(self.terminal_font.family.clone())
                            .text_size(px(self.code_font_size))
                            .h(relative(1.)),
                    )
                    .into_any_element()
            }
            (
                None,
                None,
                FileViewContent::Loading | FileViewContent::Text | FileViewContent::Image,
                _,
            ) => empty_state("Loading file…", cx),
            (None, None, FileViewContent::Binary, _) => empty_state("Binary file", cx),
            (None, None, FileViewContent::TooLarge { bytes }, _) => empty_state(
                format!(
                    "File too large to show ({:.1} MiB)",
                    *bytes as f64 / (1024. * 1024.)
                ),
                cx,
            ),
            (None, None, FileViewContent::Failed(reason), _) => failed_state(reason.clone(), cx),
        };
        v_flex()
            .debug_selector(|| "file-tab".into())
            .size_full()
            .child(header)
            .child(body)
            .into_any_element()
    }

    /// The path with its icon and Changes status, then what the Tab shows of it: an
    /// image's size, or the choice between a renderable file's rendering and its text.
    #[allow(clippy::too_many_arguments)]
    fn render_file_header(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: TabId,
        path: &RelativePath,
        rendering: Option<Rendering>,
        source: bool,
        image: Option<&Option<Result<Arc<DecodedImage>, SharedString>>>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let entry = self
            .connection(key)
            .and_then(|connection| connection.model.workspace_git.get(&workspace_id))
            .and_then(|git| git.changes.entries.iter().find(|entry| entry.path == path));
        let summary = image.and_then(|image| match image {
            Some(Ok(image)) => Some(image_summary(image)),
            _ => None,
        });
        let owner = cx.weak_entity();
        let toggle = rendering.map(|_| {
            ButtonGroup::new("file-view-mode")
                .ghost()
                .xsmall()
                .child(
                    Button::new("file-view-rendered")
                        .debug_selector(|| "file-view-rendered".into())
                        .label("Preview")
                        .selected(!source),
                )
                .child(
                    Button::new("file-view-source")
                        .debug_selector(|| "file-view-source".into())
                        .label("Source")
                        .selected(source),
                )
                .on_click(move |selected, _, cx| {
                    let source = selected.first() == Some(&1);
                    let _ = owner.update(cx, |this, cx| {
                        if let Some(editor) = this.files_view.file_editors.get_mut(&(key, tab_id)) {
                            editor.source = source;
                            cx.notify();
                        }
                    });
                })
        });
        h_flex()
            .debug_selector(|| "file-header".into())
            .flex_none()
            .h_9()
            .w_full()
            .px_3()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(type_icon(
                file_icons::file_icon_for_path(Path::new(path.as_str()), theme.is_dark()),
                false,
            ))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_sm()
                    .font_medium()
                    .child(path.to_string()),
            )
            .when_some(summary, |this, summary| {
                this.child(
                    div()
                        .debug_selector(|| "file-image-summary".into())
                        .flex_none()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(summary),
                )
            })
            .when_some(entry, |this, entry| {
                this.child(status_glyph(entry.status, cx))
            })
            .children(toggle)
            .into_any_element()
    }

    /// Brings the Preview Tab in line with the Server's answer for its file, and its
    /// images with theirs. Called from `rebuild_dock`, the one place that sees every
    /// presentation change.
    pub(super) fn sync_file_view(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: TabId,
        path: RelativePathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_file_editor(key, workspace_id, tab_id, path, window, cx);
        self.feed_images(key, workspace_id, cx);
    }

    fn sync_file_editor(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: TabId,
        path: RelativePathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.connection(key) else {
            return;
        };
        let answer = connection
            .resources
            .files
            .get(&(workspace_id, path.clone()))
            .cloned();
        if answer.is_none()
            && !connection
                .resources
                .pending_files
                .contains_key(&(workspace_id, path.clone()))
        {
            self.request_file(key, workspace_id, path.clone());
        }

        let editor = self
            .files_view
            .file_editors
            .entry((key, tab_id))
            .or_insert_with(|| {
                let state = cx.new(|cx| {
                    let mut state = EditorState::new(window, cx)
                        // Any language puts the Editor in code mode; the real one is set per
                        // file below, since one Tab shows many files over its life.
                        .language("text")
                        .line_number(true)
                        .soft_wrap(false)
                        .indent_guides(false)
                        .folding(false)
                        .searchable(true);
                    state.set_highlighter_factory(super::syntax::highlighter_factory(cx), cx);
                    state
                });
                FileEditor {
                    state,
                    shown: None,
                    content: FileViewContent::Loading,
                    markdown: None,
                    markdown_scroll: Rc::default(),
                    source: false,
                }
            });
        let Some((generation, answer)) = answer else {
            if editor
                .shown
                .as_ref()
                .is_none_or(|(shown, _)| *shown != path)
            {
                editor.content = FileViewContent::Loading;
                editor.shown = None;
                editor.markdown = None;
                editor.source = false;
                // The images of the file shown before go with it.
                let shown = (key, workspace_id, path.clone());
                release_images(self.files_view.images.retain_shown(&shown), cx);
            }
            return;
        };
        if editor.shown.as_ref() == Some(&(path.clone(), generation)) {
            if editor.content == FileViewContent::Text
                && let Some(line) = take_pending_line(
                    &mut self.files_view.pending_file_line,
                    key,
                    workspace_id,
                    &path,
                )
            {
                editor.source = true;
                editor
                    .state
                    .update(cx, |state, cx| land_on_line(state, line, window, cx));
            }
            return;
        }
        let shown_path = path.clone();
        let same_file = editor
            .shown
            .as_ref()
            .is_some_and(|(shown, _)| *shown == path);
        editor.shown = Some((path, generation));
        let (text, content) = match answer {
            Err(reason) => (String::new(), FileViewContent::Failed(reason)),
            Ok(FileContent::Image { .. }) => (String::new(), FileViewContent::Image),
            Ok(FileContent::Binary) => (String::new(), FileViewContent::Binary),
            Ok(FileContent::TooLarge { bytes }) => {
                (String::new(), FileViewContent::TooLarge { bytes })
            }
            Ok(FileContent::Text { text }) => (text, FileViewContent::Text),
        };
        let text_to_text = same_file
            && editor.content == FileViewContent::Text
            && content == FileViewContent::Text;
        let land = (content == FileViewContent::Text)
            .then(|| {
                take_pending_line(
                    &mut self.files_view.pending_file_line,
                    key,
                    workspace_id,
                    &shown_path,
                )
            })
            .flatten();
        if !same_file {
            editor.source = false;
            let shown = (key, workspace_id, shown_path.clone());
            release_images(self.files_view.images.retain_shown(&shown), cx);
        }
        if land.is_some() {
            editor.source = true;
        }
        let rendering = rendering_for(&shown_path).filter(|_| content == FileViewContent::Text);
        match (&editor.markdown, rendering) {
            // An agent editing the file the Tab shows: the view keeps where it was read.
            (Some(markdown), Some(Rendering::Markdown)) if text_to_text => {
                let list = markdown.read(cx).list_state();
                editor
                    .markdown_scroll
                    .set(Some((list.item_count(), list.logical_scroll_top())));
                markdown.update(cx, |state, cx| state.set_text(&text, cx));
            }
            (_, Some(Rendering::Markdown)) => {
                editor.markdown_scroll.set(None);
                editor.markdown = Some(cx.new(|cx| TextViewState::markdown(&text, cx)));
            }
            _ => editor.markdown = None,
        }
        if content == FileViewContent::Image || rendering == Some(Rendering::Svg) {
            self.files_view
                .images
                .want((key, workspace_id, shown_path.clone()));
        }
        editor.content = content;
        let language = language_for(&shown_path);
        editor.state.update(cx, |state, cx| {
            if text_to_text {
                // The file the Tab shows changed under it, as when an agent edits it: an
                // edit to the open document, not a new one. Kit keeps the highlighter and
                // its old tree, so the colours stay while the new parse runs, and the
                // viewport stays where it was instead of jumping to the top.
                // shortcut: each update still lands in Kit's undo history (up to 1000 steps),
                // which has no public clear; replacing only the changed span keeps a step the
                // size of the edit, not the document twice. Clear it once Kit offers a way.
                let scroll = state.scroll_offset();
                let (range, replacement) = changed_span(&state.value(), &text);
                if !range.is_empty() || !replacement.is_empty() {
                    state.set_selected_range(range, cx);
                    state.replace(replacement.to_owned(), window, cx);
                }
                state.set_selected_range(0..0, cx);
                state.set_scroll_offset(scroll, cx);
            } else {
                state.set_highlighter(language, cx);
                state.set_value(text, window, cx);
            }
            if let Some(line) = land {
                land_on_line(state, line, window, cx);
            }
        });
    }

    pub(super) fn request_directory(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: RelativePathBuf,
    ) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let (Some(server_id), Some(session_id)) =
            (connection.model.server_id, connection.model.session_id)
        else {
            return;
        };
        if connection.status != ConnectionStatus::Connected {
            return;
        }
        let request_id = connection.next_layout_request_id;
        connection.next_layout_request_id = request_id.wrapping_add(1).max(1);
        if connection.send(ClientMessage::ListDirectory {
            server_id,
            session_id,
            request_id,
            workspace_id,
            path: path.clone(),
        }) {
            connection
                .resources
                .pending_directories
                .insert((workspace_id, path), request_id);
        }
    }

    pub(super) fn request_file(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: RelativePathBuf,
    ) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let (Some(server_id), Some(session_id)) =
            (connection.model.server_id, connection.model.session_id)
        else {
            return;
        };
        if connection.status != ConnectionStatus::Connected {
            return;
        }
        let request_id = connection.next_layout_request_id;
        connection.next_layout_request_id = request_id.wrapping_add(1).max(1);
        if connection.send(ClientMessage::ReadFile {
            server_id,
            session_id,
            request_id,
            workspace_id,
            path: path.clone(),
        }) {
            connection
                .resources
                .pending_files
                .insert((workspace_id, path), request_id);
        }
    }

    /// The working tree moved: asks again for every cached listing of the Workspace and
    /// for the file its Preview Tab shows, keeping the old answers on screen until the new
    /// ones land. Every other cached file is dropped rather than refetched; a Tab that
    /// comes back to one asks for it again.
    pub(super) fn refresh_workspace_files(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
    ) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let shown = connection.session().and_then(|session| {
            Some(
                session
                    .file_tab(workspace_id)?
                    .file()?
                    .path()
                    .to_relative_path_buf(),
            )
        });
        let directories = connection
            .resources
            .refresh_files(workspace_id, shown.as_deref());
        self.files_view
            .images
            .refresh(key, workspace_id, shown.as_deref());
        for path in directories {
            self.request_directory(key, workspace_id, path);
        }
        if let Some(path) = shown {
            self.request_file(key, workspace_id, path);
        }
    }

    /// Forgets the requests in flight on `key`: their answers will never come once the
    /// connection they were sent on is gone, and a Bootstrap has just replaced every
    /// cache they would have filled. Without this a listing or file stays "pending" and
    /// is never asked for again.
    pub(super) fn clear_pending_requests(&mut self, key: ConnectionKey) {
        if let Some(connection) = self.connection_mut(key) {
            connection.resources.clear_pending();
        }
    }

    /// The same retention rules handle a closed Workspace and a replaced connection.
    pub(super) fn prune_files_state(&mut self, key: ConnectionKey, cx: &mut App) {
        let Some(connection) = self
            .connections
            .iter_mut()
            .find(|connection| connection.key == key)
        else {
            return;
        };
        release_images(
            self.files_view.retain_session(key, connection.session()),
            cx,
        );
        let live = connection
            .session()
            .into_iter()
            .flat_map(Session::workspaces)
            .map(Workspace::id)
            .collect();
        connection.resources.retain_workspaces(&live);
    }

    pub(super) fn clear_files_state(&mut self, key: ConnectionKey, cx: &mut App) {
        release_images(self.files_view.retain_session(key, None), cx);
        self.clear_pending_requests(key);
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

pub(super) fn truncated_note(cx: &App) -> AnyElement {
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
    use super::{absolute_path, changed_span, language_for, terminal_path_text};
    use relative_path::RelativePath;
    use std::path::Path;

    #[test]
    fn a_changed_file_replaces_only_the_span_that_differs() {
        let span = |old, new| {
            let (range, text) = changed_span(old, new);
            let mut patched = String::from(old);
            patched.replace_range(range.clone(), text);
            assert_eq!(patched, new);
            (range, text)
        };
        assert_eq!(span("fn a() {}\n", "fn b() {}\n"), (3..4, "b"));
        assert_eq!(span("one\nthree\n", "one\ntwo\nthree\n"), (5..5, "wo\nt"));
        assert_eq!(span("aaa", "aa"), (2..3, ""));
        assert_eq!(span("same", "same"), (4..4, ""));
        // é and è share their first byte: the span widens to the whole character.
        assert_eq!(span("caé!", "caè!"), (2..4, "è"));
        assert_eq!(span("", "新"), (0..0, "新"));
    }

    #[test]
    fn closing_workspace_clears_its_file_view_without_touching_another_device() {
        use super::{FilesViewState, Session};
        let mut session = Session::new();
        session.create_workspace(std::env::temp_dir()).unwrap();
        let workspace = &session.workspaces()[0];
        let id = workspace.id();
        let tab = workspace.tabs()[0].id();
        let mut view = FilesViewState::default();
        for key in [1, 2] {
            view.changes_open.insert((key, id));
            view.changes_against_base.insert((key, id));
            view.expanded_dirs.insert((key, id, "src".into()));
            view.last_terminal_tabs.insert((key, id), tab);
        }
        view.pending_file_line = Some((1, id, "README.md".into(), 10));
        view.retain_session(1, Some(&session));
        assert!(
            view.changes_open.contains(&(1, id)),
            "reconnect keeps live view choices"
        );
        session.close_workspace(id).unwrap();
        view.retain_session(1, Some(&session));
        assert!(!view.changes_open.contains(&(1, id)));
        assert!(!view.last_terminal_tabs.contains_key(&(1, id)));
        assert!(view.pending_file_line.is_none());
        assert!(view.changes_open.contains(&(2, id)));
        assert!(view.last_terminal_tabs.contains_key(&(2, id)));
        view.retain_session(2, None);
        assert!(view.changes_open.is_empty());
        assert!(view.changes_against_base.is_empty());
        assert!(view.expanded_dirs.is_empty());
        assert!(view.last_terminal_tabs.is_empty());
    }

    #[test]
    fn closing_the_preview_tab_lets_its_images_go() {
        use super::{FilesViewState, Session};
        let mut session = Session::new();
        session.create_workspace(std::env::temp_dir()).unwrap();
        let id = session.workspaces()[0].id();
        let preview = session.show_file(id, "logo.png".into()).unwrap();
        let mut view = FilesViewState::default();
        assert!(view.images.want((1, id, "logo.png".into())));
        view.retain_session(1, Some(&session));
        assert!(
            !view.images.want((1, id, "logo.png".into())),
            "the open Preview Tab keeps its image"
        );
        session.close_tab(preview).unwrap();
        view.retain_session(1, Some(&session));
        assert!(
            view.images.want((1, id, "logo.png".into())),
            "the closed Preview Tab's image was dropped"
        );
    }

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

    #[test]
    fn the_highlighter_follows_the_extension_and_falls_back_to_plain_text() {
        assert_eq!(
            language_for(RelativePath::new("src/main.RS")).as_ref(),
            "rs"
        );
        assert_eq!(
            language_for(RelativePath::new("Cargo.toml")).as_ref(),
            "toml"
        );
        assert_eq!(
            language_for(RelativePath::new("Dockerfile")).as_ref(),
            "text"
        );
        assert_eq!(
            language_for(RelativePath::new(".gitignore")).as_ref(),
            "text"
        );
    }
}
