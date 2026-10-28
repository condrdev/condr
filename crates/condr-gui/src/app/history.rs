//! History (ADR 0046): the presented Workspace's commits in the right sidebar, below them
//! the files a selected one changed, and one of those files in the Workspace's Diff Tab. It
//! reads only; nothing here acts on the repository.

use super::change_tree::{TreeRow, directory_row, file_row, row_highlight, tree_rows};
use super::placeholders::{empty_state, failed_state, list_note};
use super::sidebar::{upstream_counts, upstream_label};
use super::*;
use condr_core::protocol::{DiffBase, relative_age};
use condr_core::{GitChangeEntry, HISTORY_PAGE, MAX_GIT_CHANGES};
use gpui_kit::base::text::TextView;
use gpui_kit::component::resizable::{resizable_panel, v_resizable};
use gpui_kit::component::scroll::ScrollableElement as _;

/// The commit this Client's Diff Tab shows for one path, chosen in History. Any other path
/// in the Tab, whoever put it there, is the working tree's.
pub(super) struct DiffCommit {
    pub(super) path: RelativePathBuf,
    pub(super) commit: CommitSummary,
}

/// History's first row, level with Changes' comparison row: the branch the commits are
/// on and `↑N ↓N` against its upstream, as the Workspace's row in the left sidebar shows.
pub(super) fn history_branch(git: &WorkspaceGitSnapshot, cx: &App) -> AnyElement {
    h_flex()
        .flex_none()
        .w_full()
        .px_1()
        .py_1()
        .gap_2()
        .justify_between()
        .items_center()
        .text_xs()
        // `px_1`, `h_5` and the colour are an xsmall ghost Button's, so the name reads
        // like "Against HEAD" and sits where it does. Without a base that Button is
        // disabled, and Kit dims a disabled one's label so.
        .child(
            div()
                .h_5()
                .px_1()
                .flex()
                .items_center()
                .min_w_0()
                .truncate()
                .text_color(if git.base.is_some() {
                    cx.theme().secondary_foreground
                } else {
                    cx.theme().muted_foreground.opacity(0.5)
                })
                .child(git.branch.clone().unwrap_or_else(|| "detached".to_owned())),
        )
        .children(git.upstream.and_then(upstream_counts).map(|counts| {
            div()
                .debug_selector(|| "history-upstream".into())
                .flex_none()
                .pr_2()
                .child(upstream_label(&counts, cx))
        }))
        .into_any_element()
}

impl Condr {
    /// History's body: the commits loaded so far, newest first, and "Show more" while the
    /// Server says there are more; below them, while one is selected, the files it changed.
    pub(super) fn render_history(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        shown_diff: Option<&RelativePath>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(connection) = self.connection(key) else {
            return empty_state("No Workspace", cx);
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
            return empty_state("Loading history…", cx);
        };
        if log.commits.is_empty() {
            return empty_state("No commits yet", cx);
        }
        let shown = self
            .files_view
            .diff_commits
            .get(&(key, workspace_id))
            .filter(|chosen| shown_diff == Some(chosen.path.as_relative_path()))
            .map(|chosen| (chosen.commit.id.as_str(), chosen.path.as_relative_path()));
        let selected = self
            .files_view
            .selected_commits
            .get(&(key, workspace_id))
            .and_then(|id| log.commits.iter().find(|commit| commit.id == *id));
        let mut list = v_flex()
            .id("condr-history-list")
            .size_full()
            .px_1()
            .py_1()
            .overflow_y_scrollbar();
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
                    Button::new("history-more")
                        .debug_selector(|| "history-more".into())
                        .ghost()
                        .xsmall()
                        .label("Show more")
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
        let body = div().flex_1().min_h_0().w_full();
        match selected {
            Some(commit) => body.child(
                v_resizable("condr-history")
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
                    let selected = &mut this.files_view.selected_commits;
                    if selected.get(&(key, workspace_id)) == Some(&id) {
                        selected.remove(&(key, workspace_id));
                    } else {
                        selected.insert((key, workspace_id), id.clone());
                    }
                    cx.notify();
                });
            })
            .context_menu(move |menu, _, _| {
                let copy_id = copy_id.clone();
                menu.item(
                    PopupMenuItem::new("Copy Commit ID").on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy_id.clone()));
                    }),
                )
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
            return empty_state("No Workspace", cx);
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
                            this.request_commit(key, workspace_id, id);
                            cx.notify();
                        });
                    });
                }
                return empty_state("Loading…", cx);
            }
            Some(Err(error)) => return failed_state(error.clone(), cx),
            Some(Ok(details)) => details,
        };
        let mut list = v_flex()
            .id("condr-commit-files")
            .flex_1()
            .min_h_0()
            .w_full()
            .px_1()
            .pb_1()
            .overflow_y_scrollbar();
        if details.changes.truncated {
            list = list.child(list_note(
                format!("Only the first {MAX_GIT_CHANGES} files are listed."),
                warning,
            ));
        }
        if details.changes.entries.is_empty() {
            list = list.child(list_note("No file changes", muted));
        }
        let entries: Vec<&GitChangeEntry> = details.changes.entries.iter().collect();
        let folded = |path: &RelativePath| {
            self.files_view.collapsed_commit_dirs.contains(&(
                key,
                workspace_id,
                path.to_relative_path_buf(),
            ))
        };
        for row in tree_rows(&entries, folded) {
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
                    .child(format!("Changed files ({})", details.changes.entries.len())),
            )
            .child(list)
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
                        if !this.files_view.collapsed_commit_dirs.remove(&dir) {
                            this.files_view.collapsed_commit_dirs.insert(dir);
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

    /// The Diff Tab's second header row for a commit: its subject, short id, author and
    /// age, Copy Commit ID, and the rest of the message behind a disclosure.
    pub(super) fn render_commit_header(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        commit: &CommitSummary,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let body = self
            .connection(key)
            .and_then(|connection| {
                connection
                    .resources
                    .commits
                    .get(&(workspace_id, commit.id.clone()))
            })
            .and_then(|details| details.as_ref().ok())
            .map(|details| message_body(&details.message))
            .filter(|body| !body.is_empty());
        let open = self.files_view.commit_message_open;
        let owner = cx.weak_entity();
        let copy_id = commit.id.clone();
        let row = h_flex()
            .w_full()
            .gap_2()
            .items_center()
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_sm()
                    .font_medium()
                    .child(commit.subject.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(format!(
                        "{} · {} · {}",
                        short_id(&commit.id),
                        commit.author,
                        age(commit.time)
                    )),
            )
            .child(
                Button::new("diff-copy-commit")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::Copy))
                    .debug_selector(|| "diff-copy-commit".into())
                    .tooltip("Copy Commit ID")
                    .accessibility_label("Copy Commit ID")
                    .on_click(move |_, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy_id.clone()));
                    }),
            )
            .when(body.is_some(), |this| {
                this.child(
                    Button::new("diff-commit-message")
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(if open {
                            IconName::ChevronUp
                        } else {
                            IconName::ChevronDown
                        }))
                        .debug_selector(|| "diff-commit-message".into())
                        .tooltip(if open { "Hide Message" } else { "Show Message" })
                        .on_click(move |_, _, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                this.files_view.commit_message_open ^= true;
                                cx.notify();
                            });
                        }),
                )
            });
        v_flex()
            .debug_selector(|| "diff-commit-header".into())
            .flex_none()
            .w_full()
            .px_3()
            .py_1()
            .gap_1()
            .border_b_1()
            .border_color(theme.border)
            .child(row)
            .when_some(body.filter(|_| open), |this, body| {
                this.child(
                    div()
                        .id("diff-commit-body")
                        .debug_selector(|| "diff-commit-body".into())
                        .max_h(rems(12.))
                        .overflow_y_scrollbar()
                        .text_sm()
                        .child(
                            TextView::markdown(
                                SharedString::from(format!("commit-body-{}", commit.id)),
                                escaped_markdown(&body),
                            )
                            .selectable(true),
                        ),
                )
            })
            .into_any_element()
    }

    /// The commit the Diff Tab shows for `path`, when this Client chose one in History.
    pub(super) fn diff_commit(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: &RelativePath,
    ) -> Option<&CommitSummary> {
        self.files_view
            .diff_commits
            .get(&(key, workspace_id))
            .filter(|chosen| chosen.path == path)
            .map(|chosen| &chosen.commit)
    }

    /// Shows `path` as `commit` changed it in the Workspace's Diff Tab: the ordinary
    /// `ShowDiff`, with the commit as this Client's own choice (ADR 0046).
    fn show_commit_diff(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        commit: CommitSummary,
        path: RelativePathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_diff_on(key, workspace_id, path.clone(), window, cx);
        self.files_view
            .diff_commits
            .insert((key, workspace_id), DiffCommit { path, commit });
        // A Tab already on this path gets no layout change to resync it.
        self.rebuild_dock(window, cx);
        cx.notify();
    }

    /// Asks for a page of History: the first replaces what is shown once it lands, a later
    /// one walks on from where the first began.
    pub(super) fn request_log(&mut self, key: ConnectionKey, workspace_id: WorkspaceId, skip: u32) {
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
        let from = (skip > 0)
            .then(|| connection.resources.logs.get(&workspace_id)?.from.clone())
            .flatten();
        if let Some(request_id) = self.send_request(key, |server_id, session_id, request_id| {
            ClientMessage::GitLog {
                server_id,
                session_id,
                request_id,
                workspace_id,
                from,
                skip,
                limit: HISTORY_PAGE,
            }
        }) && let Some(connection) = self.connection_mut(key)
        {
            connection
                .resources
                .pending_logs
                .insert(workspace_id, (request_id, skip));
        }
    }

    /// History's first page again, once `HEAD` moved or the user refreshed; only for a
    /// Workspace whose History was loaded, and the old list stays until the new one lands.
    pub(super) fn reload_history(&mut self, key: ConnectionKey, workspace_id: WorkspaceId) {
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

    fn request_commit(&mut self, key: ConnectionKey, workspace_id: WorkspaceId, commit: String) {
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
            }
        }) && let Some(connection) = self.connection_mut(key)
        {
            connection.resources.commits.asked(slot, request_id);
        }
    }

    /// One file of one commit for the Diff Tab, with every line as context.
    pub(super) fn request_commit_diff(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        commit: String,
        path: RelativePathBuf,
    ) {
        let slot = (workspace_id, commit.clone(), path.clone());
        if self.connection(key).is_none_or(|connection| {
            connection.resources.commit_diffs.contains_key(&slot)
                || connection
                    .resources
                    .pending_commit_diffs
                    .values()
                    .any(|pending| *pending == slot)
        }) {
            return;
        }
        // The message's body for the header comes with the commit's own details.
        self.request_commit(key, workspace_id, commit.clone());
        if let Some(request_id) = self.send_request(key, |server_id, session_id, request_id| {
            ClientMessage::GitDiff {
                server_id,
                session_id,
                request_id,
                workspace_id,
                path,
                against: DiffBase::Head,
                full_context: true,
                commit: Some(commit),
            }
        }) && let Some(connection) = self.connection_mut(key)
        {
            connection
                .resources
                .pending_commit_diffs
                .insert(request_id, slot);
        }
    }

    fn send_request(
        &mut self,
        key: ConnectionKey,
        build: impl FnOnce(ServerId, SessionId, u64) -> ClientMessage,
    ) -> Option<u64> {
        self.connection_mut(key)?.send_request(build)
    }
}

pub(super) fn short_id(id: &str) -> &str {
    &id[..id.len().min(7)]
}

fn age(time: i64) -> String {
    relative_age(u64::try_from(time).unwrap_or_default())
}

/// A message without its subject line and the blank line after it.
fn message_body(message: &str) -> String {
    let mut lines = message.lines();
    lines.next();
    lines
        .skip_while(|line| line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_owned()
}

/// `text` as Markdown that renders it as written: every punctuation character escaped and
/// every line break kept, since a commit message is plain text (ADR 0046).
fn escaped_markdown(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len() * 2);
    for (index, line) in text.lines().enumerate() {
        if index > 0 {
            escaped.push_str("\\\n");
        }
        for character in line.chars() {
            if character.is_ascii_punctuation() {
                escaped.push('\\');
            }
            escaped.push(character);
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::{escaped_markdown, message_body};

    #[test]
    fn a_message_body_drops_the_subject_and_escapes_as_written() {
        assert_eq!(
            message_body("Fix the parser\n\n- keeps *tokens*\n- drops none\n"),
            "- keeps *tokens*\n- drops none"
        );
        assert_eq!(message_body("Only a subject\n"), "");
        assert_eq!(
            escaped_markdown("- keeps *tokens*\n# not a heading"),
            "\\- keeps \\*tokens\\*\\\n\\# not a heading"
        );
    }
}
