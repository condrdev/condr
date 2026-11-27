//! Git History (ADR 0046): the presented Workspace's commits, newest first, and below them
//! the files a selected one changed; clicking one shows it in the Workspace's Diff Tab. It
//! reads only; nothing here acts on the repository.

use super::change_tree::{TreeRow, directory_row, file_row, tree_rows};
use super::*;
use crate::app::format::age;
use crate::app::git_marks::short_id;
use crate::app::left_sidebar::{upstream_counts, upstream_label};
use crate::app::list_row::{row_highlight, row_menu};
use crate::app::placeholders::{empty_state, failed_state, list_note};
use condr_core::{GIT_LOG_PAGE, GitChangeEntry, MAX_GIT_CHANGES};
use gpui_kit::component::resizable::{resizable_panel, v_resizable};

/// Which commit Git History opens, in which repository, and its folds.
#[derive(Default)]
pub(in crate::app) struct GitHistoryState {
    /// The commit each Workspace's Git History lists the files of, below its commits.
    pub(in crate::app) selected_commits: HashMap<(ConnectionKey, WorkspaceId), String>,
    /// The repository each Workspace above several shows in Git History (ADR 0073); this
    /// Client's view, never remembered.
    pub(in crate::app) repositories: HashMap<(ConnectionKey, WorkspaceId), RelativePathBuf>,
    /// Directories folded shut in those files' tree, whichever commit is selected.
    pub(in crate::app) collapsed_dirs: HashSet<(ConnectionKey, WorkspaceId, RelativePathBuf)>,
}

/// The branch a repository is on, or that its `HEAD` is detached.
fn repository_branch(repository: &RepositoryGitSnapshot) -> String {
    repository
        .branch
        .clone()
        .unwrap_or_else(|| t!("git_history.detached").into_owned())
}

impl Condr {
    /// Git History's first row, level with Changes' comparison row: the branch the commits are
    /// on and `↑N ↓N` against its upstream, as the Workspace's row in the left sidebar shows.
    /// With several repositories the branch is a menu of them (ADR 0073).
    pub(in crate::app) fn render_git_history_head(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        git: &WorkspaceGitSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(shown) = self.git_history_repository(key, workspace_id, git) else {
            return div().into_any_element();
        };
        let menu =
            self.git_history_repository_menu("git-history-repository", key, workspace_id, git, cx);
        let control = menu.unwrap_or_else(|| {
            // `px_1`, `h_5` and the colour are an xsmall ghost Button's, so the name reads
            // like "Against HEAD" and sits where it does. Without a base that Button is
            // disabled, and Kit dims a disabled one's label so.
            div()
                .h_5()
                .px_1()
                .flex()
                .items_center()
                .min_w_0()
                .truncate()
                .text_color(if shown.base.is_some() {
                    cx.theme().secondary_foreground
                } else {
                    cx.theme().muted_foreground.opacity(0.5)
                })
                .child(repository_branch(shown))
                .into_any_element()
        });
        h_flex()
            .flex_none()
            .w_full()
            .px_1()
            .py_1()
            .gap_2()
            .justify_between()
            .items_center()
            .text_xs()
            .child(control)
            .children(shown.upstream.and_then(upstream_counts).map(|counts| {
                div()
                    .debug_selector(|| "git-history-upstream".into())
                    .flex_none()
                    .pr_2()
                    .child(upstream_label(&counts, cx))
            }))
            .into_any_element()
    }

    /// The menu of a Workspace's repositories, each with its branch, that Git History and
    /// Git Graph show one of (ADR 0073); none for a Workspace in one repository.
    pub(in crate::app) fn git_history_repository_menu(
        &self,
        id: &'static str,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        git: &WorkspaceGitSnapshot,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if git.repositories.len() == 1 && git.root_repository().is_some() {
            return None;
        }
        let shown = self.git_history_repository(key, workspace_id, git)?;
        let label = |repository: &RepositoryGitSnapshot| {
            let name = self.repository_name(key, workspace_id, &repository.directory);
            format!("{name} · {}", repository_branch(repository))
        };
        let choices: Vec<(RelativePathBuf, String)> = git
            .repositories
            .iter()
            .map(|repository| (repository.directory.clone(), label(repository)))
            .collect();
        let current = shown.directory.clone();
        let owner = cx.weak_entity();
        Some(
            Button::new(id)
                .debug_selector(move || id.into())
                .ghost()
                .xsmall()
                .label(label(shown))
                .dropdown_caret(true)
                .dropdown_menu(move |menu, _, _| {
                    choices.iter().fold(menu, |menu, (directory, label)| {
                        let owner = owner.clone();
                        let directory = directory.clone();
                        menu.item(
                            PopupMenuItem::new(label.clone())
                                .checked(directory == current)
                                .on_click(move |_, _, cx| {
                                    let _ = owner.update(cx, |this, cx| {
                                        this.choose_git_history_repository(
                                            key,
                                            workspace_id,
                                            directory.clone(),
                                            cx,
                                        );
                                    });
                                }),
                        )
                    })
                })
                .into_any_element(),
        )
    }

    /// The repository Git History shows: this Client's pick for the Workspace, which it does
    /// not save, else the first (ADR 0073).
    pub(in crate::app) fn git_history_repository<'a>(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        git: &'a WorkspaceGitSnapshot,
    ) -> Option<&'a RepositoryGitSnapshot> {
        self.right_sidebar
            .git_history
            .repositories
            .get(&(key, workspace_id))
            .and_then(|chosen| {
                git.repositories
                    .iter()
                    .find(|repository| repository.directory == *chosen)
            })
            .or_else(|| git.repositories.first())
    }

    /// The directory of the repository Git History shows, empty for the Root's own.
    pub(in crate::app) fn git_history_directory(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
    ) -> RelativePathBuf {
        self.connection(key)
            .and_then(|connection| connection.model.workspace_git.get(&workspace_id))
            .and_then(|git| self.git_history_repository(key, workspace_id, git))
            .map(|repository| repository.directory.clone())
            .unwrap_or_default()
    }

    /// Shows another repository's Git History: what was loaded of the last one goes, and the
    /// rebuild asks for the first page.
    pub(in crate::app) fn choose_git_history_repository(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        directory: RelativePathBuf,
        cx: &mut Context<Self>,
    ) {
        if self.git_history_directory(key, workspace_id) == directory {
            return;
        }
        self.right_sidebar
            .git_history
            .repositories
            .insert((key, workspace_id), directory);
        self.right_sidebar
            .git_history
            .selected_commits
            .remove(&(key, workspace_id));
        if let Some(connection) = self.connection_mut(key) {
            connection.resources.logs.remove(&workspace_id);
            connection.resources.pending_logs.remove(&workspace_id);
        }
        self.forget_git_graph(key, workspace_id);
        cx.notify();
    }

    /// Git History's body: the commits loaded so far, newest first, and "Show more" while the
    /// Server says there are more; below them, while one is selected, the files it changed.
    pub(in crate::app) fn render_git_history(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        shown_diff: Option<&RelativePath>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(connection) = self.connection(key) else {
            return empty_state(t!("git_history.no_workspace"), cx);
        };
        let resources = &connection.resources;
        let Some(log) = resources.logs.get(&workspace_id) else {
            if !resources.pending_logs.contains_key(&workspace_id) {
                let owner = cx.weak_entity();
                cx.defer(move |cx| {
                    let _ = owner.update(cx, |this, cx| {
                        this.request_log(key, workspace_id, 0);
                        cx.notify();
                    });
                });
            }
            return empty_state(t!("git_history.loading"), cx);
        };
        if log.commits.is_empty() {
            return empty_state(t!("git_history.no_commits"), cx);
        }
        let shown = self
            .diff_pane
            .commits
            .get(&(key, workspace_id))
            .filter(|chosen| shown_diff == Some(chosen.path.as_relative_path()))
            .map(|chosen| (chosen.commit.id.as_str(), chosen.path.as_relative_path()));
        let selected = self
            .right_sidebar
            .git_history
            .selected_commits
            .get(&(key, workspace_id))
            .and_then(|id| log.commits.iter().find(|commit| commit.id == *id));
        let mut list = v_flex().w_full().px_1().py_1();
        for commit in &log.commits {
            let is_selected = selected.is_some_and(|selected| selected.id == commit.id);
            list = list.child(self.render_commit_row(key, workspace_id, commit, is_selected, cx));
        }
        if log.more {
            let loading = resources.pending_logs.contains_key(&workspace_id);
            let owner = cx.weak_entity();
            let skip = log.commits.len() as u32;
            list = list.child(
                div().px_2().py_1().child(
                    Button::new("git-history-more")
                        .debug_selector(|| "git-history-more".into())
                        .ghost()
                        .xsmall()
                        .label(t!("git_history.show_more"))
                        .loading(loading)
                        .on_click(move |_, _, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                this.request_log(key, workspace_id, skip);
                                cx.notify();
                            });
                        }),
                ),
            );
        }
        let list = sidebar_list("condr-git-history-list", list).size_full();
        let body = div().flex_1().min_h_0().w_full();
        match selected {
            Some(commit) => body.child(
                v_resizable("condr-git-history")
                    .child(resizable_panel().child(list))
                    .child(
                        resizable_panel().child(
                            self.render_commit_files(
                                key,
                                workspace_id,
                                commit,
                                shown
                                    .filter(|(id, _)| *id == commit.id)
                                    .map(|(_, path)| path),
                                cx,
                            ),
                        ),
                    ),
            ),
            None => body.child(list),
        }
        .into_any_element()
    }

    /// Two lines: the subject, then the short id, the author and the age. A click selects
    /// the commit, and another on it lets go; its menu copies the id.
    fn render_commit_row(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        commit: &CommitSummary,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let selector: SharedString = format!("commit-{}", short_id(&commit.id)).into();
        let owner = cx.weak_entity();
        let id = commit.id.clone();
        let copy_id = commit.id.clone();
        h_flex()
            .id(selector.clone())
            .debug_selector(move || selector.to_string())
            .w_full()
            .min_w_0()
            .px_2()
            .py_1()
            .cursor_pointer()
            .rounded(theme.radius)
            .map(|row| row_highlight(row, selected, cx))
            .on_click(move |_, _, cx| {
                let _ = owner.update(cx, |this, cx| {
                    let selected = &mut this.right_sidebar.git_history.selected_commits;
                    if selected.get(&(key, workspace_id)) == Some(&id) {
                        selected.remove(&(key, workspace_id));
                    } else {
                        selected.insert((key, workspace_id), id.clone());
                    }
                    cx.notify();
                });
            })
            .map(|row| {
                let menu = move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
                    let copy_id = copy_id.clone();
                    menu.item(
                        PopupMenuItem::new(t!("git_history.copy_commit_id"))
                            .icon(menu_icon(Glyph::Copy))
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(copy_id.clone()));
                            }),
                    )
                };
                row_menu(row, selected, menu, cx)
            })
            .child(
                v_flex()
                    .min_w_0()
                    .flex_1()
                    .child(div().text_sm().truncate().child(commit.subject.clone()))
                    .child(
                        div()
                            .text_xs()
                            .truncate()
                            .text_color(theme.muted_foreground)
                            .child(format!(
                                "{} · {} · {}",
                                short_id(&commit.id),
                                commit.author,
                                age(commit.time)
                            )),
                    ),
            )
            .into_any_element()
    }

    /// The files the selected commit changed, as Changes draws its tree, under a count.
    fn render_commit_files(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        commit: &CommitSummary,
        shown: Option<&RelativePath>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (muted, warning) = {
            let theme = cx.theme();
            (theme.muted_foreground, theme.warning)
        };
        let slot = (workspace_id, commit.id.clone());
        let Some(connection) = self.connection(key) else {
            return empty_state(t!("git_history.no_workspace"), cx);
        };
        let resources = &connection.resources;
        let details = match resources.commits.get(&slot) {
            None => {
                // Asked here rather than on the click, so a reconnect that dropped the
                // answer asks again.
                if !resources.commits.is_pending(&slot) {
                    let owner = cx.weak_entity();
                    let id = commit.id.clone();
                    cx.defer(move |cx| {
                        let _ = owner.update(cx, |this, cx| {
                            let repository = this.git_history_directory(key, workspace_id);
                            this.request_commit(key, workspace_id, repository, id);
                            cx.notify();
                        });
                    });
                }
                return empty_state(t!("git_history.loading_commit"), cx);
            }
            Some(Err(error)) => return failed_state(error.clone(), cx),
            Some(Ok(details)) => details,
        };
        let mut list = v_flex().w_full().px_1().pb_1();
        if details.changes.truncated {
            list = list.child(list_note(
                t!("git_history.truncated", limit = MAX_GIT_CHANGES),
                warning,
            ));
        }
        if details.changes.entries.is_empty() {
            list = list.child(list_note(t!("git_history.no_file_changes"), muted));
        }
        let entries: Vec<&GitChangeEntry> = details.changes.entries.iter().collect();
        let folded = |path: &RelativePath| {
            self.right_sidebar.git_history.collapsed_dirs.contains(&(
                key,
                workspace_id,
                path.to_relative_path_buf(),
            ))
        };
        for row in tree_rows(&entries, &[], folded) {
            list =
                list.child(self.render_commit_file_row(key, workspace_id, commit, row, shown, cx));
        }
        v_flex()
            .size_full()
            .child(
                div()
                    .flex_none()
                    .h_7()
                    .px_2()
                    .flex()
                    .items_center()
                    .text_xs()
                    .text_color(muted)
                    .child(t!(
                        "git_history.changed_files",
                        count = details.changes.entries.len()
                    )),
            )
            .child(
                sidebar_list("condr-commit-files", list)
                    .flex_1()
                    .min_h_0()
                    .w_full(),
            )
            .into_any_element()
    }

    /// A row of a commit's files: a directory folds, a file shows in the Diff Tab as the
    /// commit changed it.
    fn render_commit_file_row(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        commit: &CommitSummary,
        row: TreeRow<'_>,
        shown: Option<&RelativePath>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let owner = cx.weak_entity();
        match row {
            TreeRow::Directory {
                label,
                path,
                depth,
                folded,
            } => directory_row(format!("commit-files-dir-{path}"), label, depth, folded, cx)
                .on_click(move |_, _, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        let dir = (key, workspace_id, path.clone());
                        if !this.right_sidebar.git_history.collapsed_dirs.remove(&dir) {
                            this.right_sidebar.git_history.collapsed_dirs.insert(dir);
                        }
                        cx.notify();
                    });
                })
                .into_any_element(),
            TreeRow::File { name, entry, depth } => {
                let id = format!("commit-file-{}-{}", short_id(&commit.id), entry.path);
                let selected = shown == Some(entry.path.as_relative_path());
                let commit = commit.clone();
                let path = entry.path.clone();
                file_row(id, name, entry, depth, selected, cx)
                    .on_click(move |_, window, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            this.show_commit_diff(
                                key,
                                workspace_id,
                                commit.clone(),
                                path.clone(),
                                window,
                                cx,
                            );
                        });
                    })
                    .into_any_element()
            }
        }
    }

    /// Shows `path` as `commit` changed it in the Workspace's Diff Tab: the ordinary
    /// `ShowDiff`, with the commit as this Client's own choice (ADR 0046).
    pub(in crate::app) fn show_commit_diff(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        commit: CommitSummary,
        path: RelativePathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_diff_on(key, workspace_id, path.clone(), window, cx);
        self.diff_pane
            .commits
            .insert((key, workspace_id), DiffCommit { path, commit });
        // A Tab already on this path gets no layout change to resync it.
        self.rebuild_dock(window, cx);
        cx.notify();
    }

    /// Asks for a page of Git History: the first replaces what is shown once it lands, a later
    /// one walks on from where the first began.
    pub(in crate::app) fn request_log(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        skip: u32,
    ) {
        let Some(connection) = self.connection(key) else {
            return;
        };
        if connection
            .resources
            .pending_logs
            .contains_key(&workspace_id)
        {
            return;
        }
        let from = match connection.resources.logs.get(&workspace_id) {
            Some(log) if skip > 0 => log.from.clone(),
            _ => Vec::new(),
        };
        let repository = self.git_history_directory(key, workspace_id);
        if let Some(request_id) = self.send_request(key, |server_id, session_id, request_id| {
            ClientMessage::GitLog {
                server_id,
                session_id,
                request_id,
                workspace_id,
                from,
                start: condr_core::LogStart::Head,
                text: None,
                skip,
                limit: GIT_LOG_PAGE,
                repository,
            }
        }) && let Some(connection) = self.connection_mut(key)
        {
            connection
                .resources
                .pending_logs
                .insert(workspace_id, (request_id, skip));
        }
    }

    /// Git History's first page again, once `HEAD` moved or the user refreshed; only for a
    /// Workspace whose Git History was loaded, and the old list stays until the new one lands.
    pub(in crate::app) fn reload_git_history(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
    ) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        if !connection.resources.logs.contains_key(&workspace_id) {
            return;
        }
        // A newer first page wins over a later page in flight.
        connection.resources.pending_logs.remove(&workspace_id);
        self.request_log(key, workspace_id, 0);
    }

    pub(in crate::app) fn request_commit(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        repository: RelativePathBuf,
        commit: String,
    ) {
        let slot = (workspace_id, commit.clone());
        if self
            .connection(key)
            .is_none_or(|connection| !connection.resources.commits.wants(&slot))
        {
            return;
        }
        if let Some(request_id) = self.send_request(key, |server_id, session_id, request_id| {
            ClientMessage::GitCommit {
                server_id,
                session_id,
                request_id,
                workspace_id,
                commit,
                repository,
            }
        }) && let Some(connection) = self.connection_mut(key)
        {
            connection.resources.commits.asked(slot, request_id);
        }
    }
}
