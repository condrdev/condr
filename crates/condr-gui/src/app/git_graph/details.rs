//! Git Graph's right column (ADR 0079): the selected commit's files above its details, as
//! IntelliJ's log has its Changes and Details.

use super::table::{label_chip, label_color, ref_labels};
use super::*;
use crate::app::diff_pane::escaped_markdown;
use crate::app::format::commit_day_and_clock;
use crate::app::right_sidebar::{TreeRow, directory_row, file_row, tree_rows};
use condr_core::{CommitDetails, GitChangeEntry};
use gpui_kit::base::text::TextView;
use gpui_kit::component::resizable::{resizable_panel, v_resizable};
use gpui_kit::component::scroll::ScrollableElement as _;
use rust_i18n::t;

/// How many branches the details name before "Show all", as IntelliJ's do.
const SHOWN_BRANCHES: usize = 6;

impl Condr {
    /// The commit `id` as the table lists it.
    fn git_graph_commit(
        &self,
        key: ConnectionKey,
        pane_id: PaneId,
        id: &str,
        cx: &App,
    ) -> Option<CommitSummary> {
        self.git_graph
            .views
            .get(&(key, pane_id))?
            .table
            .read(cx)
            .delegate()
            .rows
            .iter()
            .find(|row| row.commit.id == id)
            .map(|row| row.commit.clone())
    }

    fn request_commit_branches(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        commit: String,
    ) {
        let slot = (workspace_id, commit.clone());
        if self
            .connection(key)
            .is_none_or(|connection| !connection.resources.commit_branches.wants(&slot))
        {
            return;
        }
        let repository = self.git_history_directory(key, workspace_id);
        if let Some(request_id) = self.send_request(key, |server_id, session_id, request_id| {
            ClientMessage::GitCommitBranches {
                server_id,
                session_id,
                request_id,
                workspace_id,
                commit,
                repository,
            }
        }) && let Some(connection) = self.connection_mut(key)
        {
            connection.resources.commit_branches.asked(slot, request_id);
        }
    }

    /// The selected commit's files above its details, or a word to select one.
    pub(super) fn render_git_graph_commit(
        &self,
        view: &GitGraphView,
        key: ConnectionKey,
        pane_id: PaneId,
        refs: &[GitRef],
        current: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let column = |header: Option<AnyElement>, body: AnyElement, cx: &App| {
            v_flex()
                .size_full()
                .border_l_1()
                .border_color(cx.theme().border)
                .child(
                    column_header(cx)
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .children(header),
                )
                .child(div().flex_1().min_h_0().child(body))
                .into_any_element()
        };
        let (Some(id), Some(connection)) = (view.selected_commit.clone(), self.connection(key))
        else {
            return column(None, empty_state(t!("git_graph.select_commit"), cx), cx);
        };
        let workspace_id = view.workspace_id;
        let resources = &connection.resources;
        let slot = (workspace_id, id.clone());
        // Asked here rather than on the click, so a reconnect that dropped them asks again.
        let ask_details =
            !resources.commits.is_pending(&slot) && resources.commits.get(&slot).is_none();
        let ask_branches = resources.commit_branches.wants(&slot);
        if ask_details || ask_branches {
            let owner = cx.weak_entity();
            let id = id.clone();
            cx.defer(move |cx| {
                let _ = owner.update(cx, |this, cx| {
                    let repository = this.git_history_directory(key, workspace_id);
                    this.request_commit(key, workspace_id, repository, id.clone());
                    this.request_commit_branches(key, workspace_id, id);
                    cx.notify();
                });
            });
        }
        let details = match resources.commits.get(&slot) {
            None => {
                return column(None, empty_state(t!("git_history.loading_commit"), cx), cx);
            }
            Some(Err(error)) => {
                return column(None, failed_state(error.clone(), cx), cx);
            }
            Some(Ok(details)) => details,
        };
        let branches = resources.commit_branches.get(&slot);
        let header = h_flex()
            .w_full()
            .items_center()
            .child(div().flex_1().truncate().child(t!(
                "git_history.changed_files",
                count = details.changes.entries.len()
            )))
            .child(self.git_graph_fold_button(key, pane_id, details, false, cx))
            .child(self.git_graph_fold_button(key, pane_id, details, true, cx))
            .into_any_element();
        let body = v_resizable(SharedString::from(format!(
            "git-graph-commit-{}",
            pane_id.as_u64()
        )))
        .child(
            resizable_panel().child(self.render_git_graph_files(view, key, pane_id, details, cx)),
        )
        .child(resizable_panel().size(px(240.)).child(
            self.render_git_graph_details(view, key, pane_id, details, refs, current, branches, cx),
        ))
        .into_any_element();
        column(Some(header), body, cx)
    }

    /// Expand All, or with `fold` Collapse All, for the commit's file tree, as IntelliJ has
    /// them over its Changes.
    fn git_graph_fold_button(
        &self,
        key: ConnectionKey,
        pane_id: PaneId,
        details: &CommitDetails,
        fold: bool,
        cx: &mut Context<Self>,
    ) -> Button {
        let (id, icon, label) = if fold {
            (
                "git-graph-collapse-all",
                Glyph::ChevronsDownUp,
                t!("git_graph.collapse_all"),
            )
        } else {
            (
                "git-graph-expand-all",
                Glyph::ChevronsUpDown,
                t!("git_graph.expand_all"),
            )
        };
        // Every directory the tree draws, unfolded, is one Collapse All folds.
        let directories: Vec<RelativePathBuf> = if fold {
            let entries: Vec<&GitChangeEntry> = details.changes.entries.iter().collect();
            tree_rows(&entries, &[], |_| false)
                .into_iter()
                .filter_map(|row| match row {
                    TreeRow::Directory { path, .. } => Some(path),
                    TreeRow::File { .. } => None,
                })
                .collect()
        } else {
            Vec::new()
        };
        let owner = cx.weak_entity();
        Button::new(id)
            .debug_selector(move || id.into())
            .ghost()
            .xsmall()
            .icon(Icon::new(icon))
            .tooltip(label.clone())
            .accessibility_label(label)
            .on_click(move |_, _, cx| {
                let _ = owner.update(cx, |this, cx| {
                    if let Some(view) = this.git_graph.views.get_mut(&(key, pane_id)) {
                        view.collapsed_dirs = directories.iter().cloned().collect();
                        cx.notify();
                    }
                });
            })
    }

    /// The files the commit changed, as Git History draws them, each folder counting its
    /// files. A click selects one; a double click shows it in the Diff Pane.
    fn render_git_graph_files(
        &self,
        view: &GitGraphView,
        key: ConnectionKey,
        pane_id: PaneId,
        details: &CommitDetails,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let entries: Vec<&GitChangeEntry> = details.changes.entries.iter().collect();
        let mut list = v_flex().w_full().px_1().pb_1();
        let folded = |path: &RelativePath| view.collapsed_dirs.contains(path);
        for row in tree_rows(&entries, &[], folded) {
            let owner = cx.weak_entity();
            let row = match row {
                TreeRow::Directory {
                    label,
                    path,
                    depth,
                    folded,
                } => {
                    let count = entries
                        .iter()
                        .filter(|entry| entry.path.starts_with(&path))
                        .count();
                    let files = if count == 1 {
                        t!("git_graph.files.one", count = count)
                    } else {
                        t!("git_graph.files.other", count = count)
                    };
                    directory_row(format!("git-graph-dir-{path}"), label, depth, folded, cx)
                        .child(div().flex_none().text_xs().text_color(muted).child(files))
                        .on_click(move |_, _, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                if let Some(view) = this.git_graph.views.get_mut(&(key, pane_id))
                                    && !view.collapsed_dirs.remove(&path)
                                {
                                    view.collapsed_dirs.insert(path.clone());
                                }
                                cx.notify();
                            });
                        })
                        .into_any_element()
                }
                TreeRow::File { name, entry, depth } => {
                    let selected = view.selected_file.as_ref() == Some(&entry.path);
                    let path = entry.path.clone();
                    let id = details.id.clone();
                    file_row(
                        format!("git-graph-file-{}", entry.path),
                        name,
                        entry,
                        depth,
                        selected,
                        cx,
                    )
                    .on_click(move |event, window, cx| {
                        let (path, id) = (path.clone(), id.clone());
                        let _ = owner.update(cx, |this, cx| {
                            let Some(view) = this.git_graph.views.get_mut(&(key, pane_id)) else {
                                return;
                            };
                            view.selected_file = Some(path.clone());
                            let workspace_id = view.workspace_id;
                            if event.click_count() >= 2
                                && let Some(commit) = this.git_graph_commit(key, pane_id, &id, cx)
                            {
                                this.show_commit_diff(key, workspace_id, commit, path, window, cx);
                            }
                            cx.notify();
                        });
                    })
                    .into_any_element()
                }
            };
            list = list.child(row);
        }
        sidebar_list(
            SharedString::from(format!("git-graph-files-{}", pane_id.as_u64())),
            list,
        )
        .size_full()
        .into_any_element()
    }

    /// The commit's message, who made it and when, its refs, and the branches it is on.
    #[allow(clippy::too_many_arguments)]
    fn render_git_graph_details(
        &self,
        view: &GitGraphView,
        key: ConnectionKey,
        pane_id: PaneId,
        details: &CommitDetails,
        refs: &[GitRef],
        current: Option<&str>,
        branches: Option<&Result<Vec<String>, String>>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let (day, clock) = commit_day_and_clock(details.time);
        let authored = t!(
            "git_graph.details.authored",
            id = short_id(&details.id),
            author = details.author,
            email = details.author_email,
            date = day,
            time = clock
        );
        let committed = details.committer.as_ref().map(|committer| {
            let (day, clock) = commit_day_and_clock(committer.time);
            t!(
                "git_graph.details.committed",
                name = committer.name,
                date = day,
                time = clock
            )
        });
        let (labels, _) = ref_labels(refs, current)
            .remove(&details.id)
            .unwrap_or_default();
        let tags: Vec<&str> = refs
            .iter()
            .filter(|reference| {
                reference.commit == details.id && reference.name.starts_with("refs/tags/")
            })
            .map(|reference| short_name(&reference.name))
            .collect();
        let tag_color = label_color(RefKind::Tag, cx);
        let branches = match branches {
            None => div()
                .child(t!("git_graph.details.branches_loading"))
                .into_any_element(),
            Some(Err(error)) => div()
                .text_color(cx.theme().danger)
                .child(error.clone())
                .into_any_element(),
            Some(Ok(names)) => {
                let shown: Vec<&str> = names
                    .iter()
                    .take(if view.all_branches {
                        names.len()
                    } else {
                        SHOWN_BRANCHES
                    })
                    .map(|name| short_name(name))
                    .collect();
                let text = if names.len() == 1 {
                    t!(
                        "git_graph.details.in_branches.one",
                        branches = shown.join(", ")
                    )
                } else {
                    t!(
                        "git_graph.details.in_branches.other",
                        count = names.len(),
                        branches = shown.join(", ")
                    )
                };
                let owner = cx.weak_entity();
                v_flex()
                    .items_start()
                    .gap_1()
                    .child(div().child(text))
                    .when(!view.all_branches && names.len() > SHOWN_BRANCHES, |this| {
                        this.child(
                            Button::new("git-graph-all-branches")
                                .debug_selector(|| "git-graph-all-branches".into())
                                .link()
                                .xsmall()
                                .label(t!("git_graph.details.show_all"))
                                .on_click(move |_, _, cx| {
                                    let _ = owner.update(cx, |this, cx| {
                                        if let Some(view) =
                                            this.git_graph.views.get_mut(&(key, pane_id))
                                        {
                                            view.all_branches = true;
                                        }
                                        cx.notify();
                                    });
                                }),
                        )
                    })
                    .into_any_element()
            }
        };
        v_flex()
            .id(SharedString::from(format!(
                "git-graph-details-{}",
                pane_id.as_u64()
            )))
            .debug_selector(|| "git-graph-details".into())
            .size_full()
            .overflow_y_scrollbar()
            .border_t_1()
            .border_color(cx.theme().border)
            .p_3()
            .gap_2()
            .text_sm()
            .child(
                TextView::markdown(
                    SharedString::from(format!("git-graph-message-{}", details.id)),
                    escaped_markdown(details.message.trim_end()),
                )
                .selectable(true),
            )
            .child(div().text_xs().text_color(muted).child(authored))
            .children(committed.map(|committed| div().text_xs().text_color(muted).child(committed)))
            .when(!labels.is_empty() || !tags.is_empty(), |this| {
                this.child(
                    h_flex()
                        .flex_wrap()
                        .gap_2()
                        .children(labels.iter().map(|label| label_chip(label, cx)))
                        .children(tags.into_iter().map(|tag| {
                            h_flex()
                                .gap_1()
                                .items_center()
                                .text_xs()
                                .text_color(tag_color)
                                .child(Icon::new(Glyph::Tag).size_3().text_color(tag_color))
                                .child(tag.to_owned())
                        })),
                )
            })
            .child(div().text_xs().text_color(muted).child(branches))
            .into_any_element()
    }
}
