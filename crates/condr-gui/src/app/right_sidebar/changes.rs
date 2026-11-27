//! The Changes view (ADR 0017): the presented Workspace's working-tree changes as the
//! Server reports them, against `HEAD` or the Base Branch (ADR 0034), each repository a
//! node of its own (ADR 0073). Everything shown here is Server data; the Client only asks
//! and draws.

use super::change_tree::{TreeRow, directory_row, file_row, repository_row, tree_rows};
use super::*;
use crate::app::git_marks::diff_stat;
use crate::app::list_row::{disclosure, row_highlight, row_menu};
use crate::app::placeholders::list_note;
use condr_core::protocol::DiffBase;
use condr_core::{GitChangeEntry, GitChangeStatus, GitChanges, MAX_GIT_CHANGES};
use std::borrow::Cow;

/// The three sections of the list, in display order, as Zed's git panel groups them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(in crate::app) enum ChangesSection {
    Conflicts,
    Tracked,
    Untracked,
}

impl ChangesSection {
    const ORDER: [Self; 3] = [Self::Conflicts, Self::Tracked, Self::Untracked];

    fn of(status: GitChangeStatus) -> Self {
        match status {
            GitChangeStatus::Conflicted => Self::Conflicts,
            GitChangeStatus::Untracked => Self::Untracked,
            GitChangeStatus::Added
            | GitChangeStatus::Modified
            | GitChangeStatus::Deleted
            | GitChangeStatus::Renamed => Self::Tracked,
        }
    }

    /// The section's header, with how many rows it holds.
    fn title(self, count: usize) -> Cow<'static, str> {
        match self {
            Self::Conflicts => t!("changes.section.conflicts", count = count),
            Self::Tracked => t!("changes.section.tracked", count = count),
            Self::Untracked => t!("changes.section.untracked", count = count),
        }
    }

    fn selector(self) -> &'static str {
        match self {
            Self::Conflicts => "changes-conflicts",
            Self::Tracked => "changes-tracked",
            Self::Untracked => "changes-untracked",
        }
    }
}

/// How this Client reads each Workspace's changes.
#[derive(Default)]
pub(in crate::app) struct ChangesState {
    /// The Workspaces whose Changes and Diff Tab compare against the base (ADR 0034); the
    /// others compare against `HEAD`. This Client's view, remembered in the state file.
    pub(in crate::app) against_base: HashSet<(ConnectionKey, WorkspaceId)>,
    pub(in crate::app) collapsed_sections: HashSet<ChangesSection>,
    /// Directories folded shut in the Changes tree, by connection, Workspace, section and
    /// Root-relative path; Workspace ids repeat across Servers, and a directory with both a
    /// tracked change and a new file is a row in two sections.
    pub(in crate::app) collapsed_dirs:
        HashSet<(ConnectionKey, WorkspaceId, ChangesSection, RelativePathBuf)>,
}

impl Condr {
    fn toggle_changes_section(&mut self, section: ChangesSection, cx: &mut Context<Self>) {
        if !self
            .right_sidebar
            .changes
            .collapsed_sections
            .remove(&section)
        {
            self.right_sidebar
                .changes
                .collapsed_sections
                .insert(section);
        }
        cx.notify();
    }

    /// What a Workspace's Changes and Diff Tab compare against (ADR 0034): the base when
    /// this Client chose it and the Server reports one, else `HEAD`.
    pub(in crate::app) fn diff_base(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        git: &WorkspaceGitSnapshot,
    ) -> DiffBase {
        if git
            .root_repository()
            .is_some_and(|repository| repository.base.is_some())
            && self
                .right_sidebar
                .changes
                .against_base
                .contains(&(key, workspace_id))
        {
            DiffBase::MergeBase
        } else {
            DiffBase::Head
        }
    }

    /// What a Workspace's Changes and Diff Tabs compare against, HEAD without Git.
    pub(in crate::app) fn workspace_diff_base(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
    ) -> DiffBase {
        self.connection(key)
            .and_then(|connection| connection.model.workspace_git.get(&workspace_id))
            .map_or(DiffBase::Head, |git| self.diff_base(key, workspace_id, git))
    }

    /// Switches what the Workspace's Changes compare against. The Diff Tab shows the other
    /// side from now on, so its answers are dropped and asked for again on the rebuild.
    pub(in crate::app) fn compare_changes_against(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        against: DiffBase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let changed = match against {
            DiffBase::MergeBase => self
                .right_sidebar
                .changes
                .against_base
                .insert((key, workspace_id)),
            DiffBase::Head => self
                .right_sidebar
                .changes
                .against_base
                .remove(&(key, workspace_id)),
        };
        if !changed {
            return;
        }
        self.forget_workspace_diffs(key, workspace_id);
        self.schedule_state_save(cx);
        self.rebuild_dock(window, cx);
        cx.notify();
    }

    /// What the Changes list compares against (ADR 0034), named on a button whose menu
    /// offers `HEAD` and the base branch the Server names, the current one checked. Without
    /// a base there is nothing to choose, so the button names `HEAD` and is disabled.
    pub(in crate::app) fn render_changes_comparison(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        git: &WorkspaceGitSnapshot,
        against: DiffBase,
        total: Option<GitDiffStat>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let button = Button::new("changes-comparison")
            .debug_selector(|| "changes-comparison".into())
            .ghost()
            .xsmall()
            .label(comparison_label(git, against));
        let base = git
            .root_repository()
            .and_then(|repository| repository.base.as_ref());
        let control = match base.map(|base| base.branch.clone()) {
            None => button
                .disabled(true)
                .tooltip(t!("changes.no_base"))
                .into_any_element(),
            Some(branch) => {
                let owner = cx.weak_entity();
                button
                    .dropdown_caret(true)
                    .dropdown_menu(move |menu, _, _| {
                        [
                            (DiffBase::Head, "HEAD".to_owned()),
                            (DiffBase::MergeBase, branch.clone()),
                        ]
                        .into_iter()
                        .fold(menu, |menu, (choice, label)| {
                            let owner = owner.clone();
                            menu.item(
                                PopupMenuItem::new(label)
                                    .checked(choice == against)
                                    .on_click(move |_, window, cx| {
                                        let _ = owner.update(cx, |this, cx| {
                                            this.compare_changes_against(
                                                key,
                                                workspace_id,
                                                choice,
                                                window,
                                                cx,
                                            );
                                        });
                                    }),
                            )
                        })
                    })
                    .into_any_element()
            }
        };
        h_flex()
            .flex_none()
            .w_full()
            // The list's gutter: the hover box lines up with the rows', and the button's own
            // padding puts its label where the section chevrons start.
            .px_1()
            .py_1()
            .gap_2()
            .justify_between()
            .items_center()
            .child(control)
            // The summed `+N −N` of what this comparison shows, the way Zed totals a branch;
            // `pr_2` inside the row's `px_1` lines it up with the rows' own stats.
            .when_some(total, |this, total| {
                this.child(div().pr_2().child(diff_stat(total, cx)))
            })
            .into_any_element()
    }

    /// What a repository goes by in Changes and Git History: its directory under the Root, or
    /// the Workspace's name for the Root's own (ADR 0073).
    pub(in crate::app) fn repository_name(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        directory: &RelativePath,
    ) -> String {
        if !directory.as_str().is_empty() {
            return directory.to_string();
        }
        self.connection(key)
            .and_then(|connection| connection.session()?.workspace(workspace_id))
            .map(|workspace| workspace.name().to_owned())
            .unwrap_or_default()
    }

    /// The Changes view's body: the sections and their trees, each repository a node of its
    /// own when there are several or it lies under the Root (ADR 0073), or why there are
    /// none.
    pub(in crate::app) fn render_changes_list(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        git: &WorkspaceGitSnapshot,
        against: DiffBase,
        shown: Option<RelativePathBuf>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Copied out so the rows below can take `cx` again.
        let theme = cx.theme();
        let (radius, muted, warning) = (theme.radius, theme.muted_foreground, theme.warning);
        let entries: Vec<&GitChangeEntry> = shown_entries(git, against).collect();
        if entries.is_empty() {
            return empty_state(t!("changes.none"), cx);
        }
        let nodes: Vec<&RelativePath> = git
            .repositories
            .iter()
            .map(|repository| repository.directory.as_relative_path())
            .filter(|directory| !directory.as_str().is_empty() || git.repositories.len() > 1)
            .collect();

        let mut list = v_flex().w_full().px_1().pb_1();
        if git
            .repositories
            .iter()
            .any(|repository| shown_changes(repository, against).truncated)
        {
            list = list.child(list_note(
                t!("changes.truncated", count = MAX_GIT_CHANGES),
                warning,
            ));
        }
        for section in ChangesSection::ORDER {
            let rows: Vec<&GitChangeEntry> = entries
                .iter()
                .copied()
                .filter(|entry| ChangesSection::of(entry.status) == section)
                .collect();
            if rows.is_empty() {
                continue;
            }
            let collapsed = self
                .right_sidebar
                .changes
                .collapsed_sections
                .contains(&section);
            let toggle_owner = cx.weak_entity();
            list = list.child(
                h_flex()
                    .id(section.selector())
                    .debug_selector(move || section.selector().into())
                    .h_7()
                    .w_full()
                    .px_1()
                    .gap_1()
                    .items_center()
                    .cursor_pointer()
                    .rounded(radius)
                    .map(|row| row_highlight(row, false, cx))
                    .on_click(move |_, _, cx| {
                        let _ = toggle_owner.update(cx, |this, cx| {
                            this.toggle_changes_section(section, cx);
                        });
                    })
                    .child(disclosure(!collapsed, cx))
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .child(section.title(rows.len())),
                    ),
            );
            if collapsed {
                continue;
            }
            let folded = |path: &RelativePath| {
                self.right_sidebar.changes.collapsed_dirs.contains(&(
                    key,
                    workspace_id,
                    section,
                    path.to_relative_path_buf(),
                ))
            };
            for mut row in tree_rows(&rows, &nodes, folded) {
                let node = match &mut row {
                    TreeRow::Directory {
                        label,
                        path,
                        depth: 0,
                        ..
                    } => git
                        .repositories
                        .iter()
                        .find(|repository| repository.directory == *path)
                        .map(|repository| {
                            *label = self.repository_name(key, workspace_id, path);
                            let count = shown_changes(repository, against)
                                .entries
                                .iter()
                                .filter(|entry| ChangesSection::of(entry.status) == section)
                                .count();
                            (count, repository.branch.clone())
                        }),
                    _ => None,
                };
                list = list.child(self.render_change_row(
                    key,
                    workspace_id,
                    section,
                    row,
                    node,
                    shown.as_deref(),
                    cx,
                ));
            }
        }
        sidebar_list("condr-changes-list", list)
            .flex_1()
            .min_h_0()
            .w_full()
            .into_any_element()
    }

    /// A Changes row: a directory folds, a file opens in the Diff Tab and has the file menu.
    /// `node` is a repository's file count and branch when the directory is its node.
    #[allow(clippy::too_many_arguments)]
    fn render_change_row(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        section: ChangesSection,
        row: TreeRow<'_>,
        node: Option<(usize, Option<String>)>,
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
            } => {
                // The section is part of the id: the same directory in two sections is two
                // rows, and rows sharing an id would share one click state.
                let id = format!("{}-dir-{path}", section.selector());
                match node {
                    Some((count, branch)) => {
                        repository_row(id, label, folded, count, branch.as_deref(), cx)
                    }
                    None => directory_row(id, label, depth, folded, cx),
                }
                .on_click(move |_, _, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        let dir = (key, workspace_id, section, path.clone());
                        if !this.right_sidebar.changes.collapsed_dirs.remove(&dir) {
                            this.right_sidebar.changes.collapsed_dirs.insert(dir);
                        }
                        cx.notify();
                    });
                })
                .into_any_element()
            }
            TreeRow::File { name, entry, depth } => {
                let selected = shown == Some(entry.path.as_relative_path());
                let deleted = entry.status == GitChangeStatus::Deleted;
                let path = entry.path.clone();
                file_row(
                    format!("change-{}", entry.path),
                    name,
                    entry,
                    depth,
                    selected,
                    cx,
                )
                .on_click(move |_, window, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        this.show_diff_on(key, workspace_id, path.clone(), window, cx);
                    });
                })
                .map(|row| {
                    let menu =
                        self.file_context_menu(key, workspace_id, &entry.path, Some(!deleted), cx);
                    row_menu(row, selected, menu, cx)
                })
                .into_any_element()
            }
        }
    }
}

/// The change list a comparison shows of a repository; `HEAD`'s when the Server reports no
/// base.
pub(in crate::app) fn shown_changes(
    repository: &RepositoryGitSnapshot,
    against: DiffBase,
) -> &GitChanges {
    match (against, &repository.base) {
        (DiffBase::MergeBase, Some(base)) => &base.changes,
        _ => &repository.changes,
    }
}

/// What a comparison shows of every repository of the Workspace, in their order.
pub(in crate::app) fn shown_entries(
    git: &WorkspaceGitSnapshot,
    against: DiffBase,
) -> impl Iterator<Item = &GitChangeEntry> {
    git.repositories
        .iter()
        .flat_map(move |repository| &shown_changes(repository, against).entries)
}

/// What the Changes header and the Diff Tab header say a comparison is against; only the
/// repository the Root is in has a base.
pub(in crate::app) fn comparison_label(
    git: &WorkspaceGitSnapshot,
    against: DiffBase,
) -> Cow<'static, str> {
    let base = git
        .root_repository()
        .and_then(|repository| repository.base.as_ref());
    match (against, base) {
        (DiffBase::MergeBase, Some(base)) => t!("changes.comparison.base", branch = base.branch),
        _ => t!("changes.comparison.head"),
    }
}

#[cfg(test)]
mod tests {
    use super::ChangesSection;
    use condr_core::GitChangeStatus;
    #[test]
    fn statuses_land_in_zed_sections() {
        assert_eq!(
            ChangesSection::of(GitChangeStatus::Conflicted),
            ChangesSection::Conflicts
        );
        assert_eq!(
            ChangesSection::of(GitChangeStatus::Untracked),
            ChangesSection::Untracked
        );
        for status in [
            GitChangeStatus::Added,
            GitChangeStatus::Modified,
            GitChangeStatus::Deleted,
            GitChangeStatus::Renamed,
        ] {
            assert_eq!(ChangesSection::of(status), ChangesSection::Tracked);
        }
    }
}
