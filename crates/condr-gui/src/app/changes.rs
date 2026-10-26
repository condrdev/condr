//! The Changes sidebar and the Diff Tab (ADR 0017): the presented Workspace's working-tree
//! changes as the Server reports them, and one file's diff in GPUI Kit's `Diff` (ADR 0046).
//! Everything shown here is Server data; the Client only asks and draws.

use super::change_tree::{
    TreeRow, diff_stat, directory_row, disclosure, file_row, row_highlight, status_glyph, tree_rows,
};
use super::history::history_branch;
use super::sidebar::CondrIconName;
use super::*;
use condr_core::protocol::DiffBase;
use condr_core::{
    DiffHunk, DiffLineKind, FileDiffContent, GitChangeEntry, GitChangeStatus, GitChanges,
    GitDiffStat, MAX_GIT_CHANGES,
};
use gpui_kit::component::button::ButtonGroup;
use gpui_kit::component::diff::{Diff, DiffFile, DiffLinePosition, DiffMode, DiffSide, DiffState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tab::{Tab, TabBar};

/// What the right sidebar shows (ADR 0018); remembered per Workspace (ADR 0023).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SidebarView {
    #[default]
    Changes,
    Files,
    /// The commits reachable from `HEAD` (ADR 0046).
    History,
}

pub(super) const INITIAL_CHANGES_WIDTH: Pixels = px(320.);
pub(super) const MIN_CHANGES_WIDTH: Pixels = px(200.);
pub(super) const MAX_CHANGES_WIDTH: Pixels = px(480.);

/// Dragging the Changes sidebar's left edge resizes it.
pub(super) struct DraggedChanges;

/// The three sections of the list, in display order, as Zed's git panel groups them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum ChangesSection {
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

    fn title(self) -> &'static str {
        match self {
            Self::Conflicts => "Conflicts",
            Self::Tracked => "Tracked",
            Self::Untracked => "Untracked",
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

/// The Diff Tab's Kit `Diff` and what it currently shows (ADR 0046).
pub(super) struct DiffViewer {
    pub(super) state: Entity<DiffState>,
    /// The path, the commit when History chose one, and the connection's diff generation
    /// the state was built from.
    shown: Option<((RelativePathBuf, Option<String>), u64)>,
    /// The patch last handed to the state. A refetch that writes the same one leaves the
    /// view alone, since `set_files` resets its scrolling and folds.
    pub(super) patch: String,
    pub(super) content: DiffContent,
    /// The line last clicked: where Show File lands, and where a changed diff scrolls back.
    pub(super) clicked: Option<DiffLinePosition>,
}

/// The one name every patch gives its file. The Tab header shows the path, and Kit's
/// parser then never meets a path that needs quoting.
pub(super) const PATCH_PATH: &str = "file";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum DiffContent {
    Loading,
    Text,
    /// Both sides are identical: the file left the change list.
    Unchanged,
    Binary,
    TooLarge {
        bytes: u64,
    },
    Failed(String),
}

impl Condr {
    /// Each Workspace keeps its own right sidebar: opening it here leaves the others
    /// closed, and coming back finds it the way it was left.
    pub(super) fn toggle_changes_sidebar(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.presented_workspace() else {
            return;
        };
        if !self.files_view.changes_open.remove(&workspace) {
            self.files_view.changes_open.insert(workspace);
        }
        self.schedule_state_save(cx);
        cx.notify();
    }

    pub(super) fn changes_sidebar_open(&self) -> bool {
        self.presented_workspace()
            .is_some_and(|workspace| self.files_view.changes_open.contains(&workspace))
    }

    /// The Workspace presented for the sidebar to review. Without one the column stays
    /// hidden and its toggle disabled.
    pub(super) fn presented_workspace(&self) -> Option<(ConnectionKey, WorkspaceId)> {
        let connection = self.active_connection()?;
        let session = connection.presented_session()?;
        let workspace_id = self.presented_workspace_id(connection.key, session)?;
        Some((connection.key, workspace_id))
    }

    fn toggle_changes_section(&mut self, section: ChangesSection, cx: &mut Context<Self>) {
        if !self.files_view.collapsed_changes_sections.remove(&section) {
            self.files_view.collapsed_changes_sections.insert(section);
        }
        cx.notify();
    }

    /// Shows `path`'s diff in the Workspace's Diff Tab: a layout command like any other,
    /// presented once the Server confirms it.
    pub(super) fn show_diff_on(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: RelativePathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The working tree's, unless History puts its commit back right after (ADR 0046).
        let left_commit = self
            .files_view
            .diff_commits
            .remove(&(key, workspace_id))
            .is_some();
        self.send_presenting_layout_to(
            key,
            LayoutCommand::ShowDiff { workspace_id, path },
            window,
            cx,
        );
        // Retargeting the Tab to its own path changes no layout, so nothing else resyncs it.
        if left_commit {
            self.rebuild_dock(window, cx);
        }
    }

    /// The title bar's toggle for the right sidebar, next to "Open in". Only a Workspace
    /// has anything to show, so the welcome page renders none.
    pub(super) fn render_changes_toggle(&self, cx: &mut Context<Self>) -> AnyElement {
        let owner = cx.weak_entity();
        let label = if self.changes_sidebar_open() {
            "Hide Changes & Files"
        } else {
            "Show Changes & Files"
        };
        let icon = if self.changes_sidebar_open() {
            IconName::PanelRightClose
        } else {
            IconName::PanelRightOpen
        };
        h_flex()
            .flex_none()
            .h_full()
            .items_center()
            .pl_2()
            // The same gutter the sidebar keeps before the window edge.
            .pr_3()
            // Inside the title bar an unclaimed press starts a window move on Windows.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                Button::new("toggle-changes")
                    .debug_selector(|| "toggle-changes".into())
                    .ghost()
                    .small()
                    .icon(Icon::new(icon))
                    .tooltip_with_action(label, &ToggleChanges, Some(SHORTCUT_CONTEXT))
                    .accessibility_label(label)
                    .on_click(move |_, _, cx| {
                        let _ = owner.update(cx, |this, cx| this.toggle_changes_sidebar(cx));
                    }),
            )
            .into_any_element()
    }

    /// The user's pick for this Workspace; a Workspace outside any repository opens on
    /// Files, since its Changes view has nothing to show (ADR 0018).
    fn sidebar_view_for(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        in_repository: bool,
    ) -> SidebarView {
        self.files_view
            .sidebar_views
            .get(&(key, workspace_id))
            .copied()
            .unwrap_or(if in_repository {
                SidebarView::Changes
            } else {
                SidebarView::Files
            })
    }

    fn set_sidebar_view(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        view: SidebarView,
        cx: &mut Context<Self>,
    ) {
        if self
            .files_view
            .sidebar_views
            .insert((key, workspace_id), view)
            != Some(view)
        {
            self.schedule_state_save(cx);
            cx.notify();
        }
    }

    /// The right sidebar: the presented Workspace's changes against `HEAD`, or its files
    /// (ADR 0018), chosen by the tabs in the header.
    pub(super) fn render_right_sidebar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let column = v_flex()
            .debug_selector(|| "condr-right-sidebar".into())
            .size_full()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_l_1()
            .border_color(theme.sidebar_border);

        let presented = self.active_connection().and_then(|connection| {
            let session = connection.session()?;
            let workspace_id = self.presented_workspace_id(connection.key, session)?;
            let workspace = session.workspace(workspace_id)?;
            let presented_tab = self.presented_tab_id(connection.key, session, workspace_id);
            let shown_diff = session
                .diff_tab(workspace_id)
                .filter(|tab| presented_tab == Some(tab.id()))
                .and_then(|tab| tab.diff().map(|diff| diff.path().to_relative_path_buf()));
            let shown_file = session
                .file_tab(workspace_id)
                .filter(|tab| presented_tab == Some(tab.id()))
                .and_then(|tab| tab.file().map(|file| file.path().to_relative_path_buf()));
            Some((
                connection.key,
                workspace_id,
                workspace.name().to_owned(),
                connection.model.workspace_git.get(&workspace_id).cloned(),
                shown_diff,
                shown_file,
            ))
        });
        let Some((key, workspace_id, workspace_name, git, shown_diff, shown_file)) = presented
        else {
            return column
                .child(self.sidebar_header(None, SidebarView::Changes, None, false, cx))
                .child(empty_state("No Workspace", cx))
                .into_any_element();
        };
        // A Server that sends `head` serves History (ADR 0046).
        let history = git.as_ref().is_some_and(|git| git.head.is_some());
        let view = match self.sidebar_view_for(key, workspace_id, git.is_some()) {
            SidebarView::History if !history => SidebarView::Changes,
            view => view,
        };
        let against = git
            .as_ref()
            .map_or(DiffBase::Head, |git| self.diff_base(key, workspace_id, git));
        let shown = git.as_ref().map(|git| shown_changes(git, against));
        let count = shown.map(|changes| changes.entries.len());
        let total = shown.and_then(|changes| {
            changes
                .entries
                .iter()
                .filter_map(|entry| entry.stat)
                .reduce(|sum, stat| GitDiffStat {
                    added: sum.added + stat.added,
                    deleted: sum.deleted + stat.deleted,
                })
        });
        let header = self.sidebar_header(Some((key, workspace_id)), view, count, history, cx);
        let body = match view {
            SidebarView::Files => {
                self.render_files_list(key, workspace_id, shown_file.as_deref(), cx)
            }
            SidebarView::Changes => match &git {
                Some(git) => v_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(self.render_changes_comparison(
                        key,
                        workspace_id,
                        git,
                        against,
                        total,
                        cx,
                    ))
                    .child(self.render_changes_list(
                        key,
                        workspace_id,
                        shown_changes(git, against),
                        shown_diff,
                        cx,
                    ))
                    .into_any_element(),
                None => empty_state(format!("{workspace_name} is not a Git repository"), cx),
            },
            SidebarView::History => v_flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .children(git.as_ref().map(|git| history_branch(git, cx)))
                .child(self.render_history(key, workspace_id, shown_diff.as_deref(), cx))
                .into_any_element(),
        };
        column.child(header).child(body).into_any_element()
    }

    /// The Files / Changes / History tabs, sized to their labels, with the change count in
    /// the Changes tab and Refresh at the far end whichever view is shown.
    fn sidebar_header(
        &self,
        target: Option<(ConnectionKey, WorkspaceId)>,
        view: SidebarView,
        count: Option<usize>,
        history: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // GPUI Kit's own tab strip, as the Kit gallery shows it; the count rides in the
        // Changes tab and Refresh takes the strip's free end.
        let owner = cx.weak_entity();
        let selected = match view {
            SidebarView::Files => 0,
            SidebarView::Changes => 1,
            SidebarView::History => 2,
        };
        let refresh = target.map(|(key, workspace_id)| {
            let owner = cx.weak_entity();
            Button::new("sidebar-refresh")
                .ghost()
                .xsmall()
                .icon(Icon::new(IconName::RefreshCw))
                .debug_selector(|| "sidebar-refresh".into())
                .tooltip("Refresh")
                .accessibility_label("Refresh")
                .loading(
                    self.files_view
                        .refreshing
                        .contains_key(&(key, workspace_id)),
                )
                .on_click(move |_, _, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        this.refresh_workspace(key, workspace_id, cx);
                    });
                })
        });
        let suffix = h_flex().pr_2().items_center().children(refresh);
        TabBar::new("condr-sidebar-view")
            .selected_index(selected)
            .on_click(move |index, _, cx| {
                let view = match *index {
                    1 => SidebarView::Changes,
                    2 => SidebarView::History,
                    _ => SidebarView::Files,
                };
                let _ = owner.update(cx, |this, cx| {
                    if let Some((key, workspace_id)) = target {
                        this.set_sidebar_view(key, workspace_id, view, cx);
                    }
                });
            })
            .children([
                Tab::new()
                    .label("Files")
                    .debug_selector(|| "sidebar-view-files".into()),
                Tab::new()
                    .label(match count {
                        Some(count) => format!("Changes ({count})"),
                        None => "Changes".to_owned(),
                    })
                    .debug_selector(|| "sidebar-view-changes".into()),
            ])
            .when(history, |this| {
                this.child(
                    Tab::new()
                        .label("History")
                        .debug_selector(|| "sidebar-view-history".into()),
                )
            })
            .suffix(suffix)
            .into_any_element()
    }

    /// The sidebar's refresh button (ADR 0046): asks the Server to rescan the Workspace,
    /// for when its watcher missed something. The button spins until the scan's
    /// `WorkspaceFilesChanged` arrives, or for five seconds when none does: a failed scan
    /// or an older Server, which answers with an error instead.
    fn refresh_workspace(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let (Some(server_id), Some(session_id)) =
            (connection.model.server_id, connection.model.session_id)
        else {
            return;
        };
        if connection.status != ConnectionStatus::Connected
            || !connection.send(ClientMessage::RefreshWorkspace {
                server_id,
                session_id,
                workspace_id,
            })
        {
            return;
        }
        let timeout = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(5))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.files_view.refreshing.remove(&(key, workspace_id));
                cx.notify();
            });
        });
        self.files_view
            .refreshing
            .insert((key, workspace_id), timeout);
        self.reload_history(key, workspace_id);
        cx.notify();
    }

    /// What a Workspace's Changes and Diff Tab compare against (ADR 0034): the base when
    /// this Client chose it and the Server reports one, else `HEAD`.
    pub(super) fn diff_base(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        git: &WorkspaceGitSnapshot,
    ) -> DiffBase {
        if git.base.is_some()
            && self
                .files_view
                .changes_against_base
                .contains(&(key, workspace_id))
        {
            DiffBase::MergeBase
        } else {
            DiffBase::Head
        }
    }

    /// Switches what the Workspace's Changes compare against. The Diff Tab shows the other
    /// side from now on, so its answers are dropped and asked for again on the rebuild.
    pub(super) fn compare_changes_against(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        against: DiffBase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let changed = match against {
            DiffBase::MergeBase => self
                .files_view
                .changes_against_base
                .insert((key, workspace_id)),
            DiffBase::Head => self
                .files_view
                .changes_against_base
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

    /// Every diff of the Workspace is stale: a Diff Tab showing one asks again on the
    /// rebuild.
    pub(super) fn forget_workspace_diffs(&mut self, key: ConnectionKey, workspace_id: WorkspaceId) {
        if let Some(connection) = self.connection_mut(key) {
            connection.resources.invalidate_diffs(workspace_id);
        }
    }

    /// What the Changes list compares against (ADR 0034), named on a button whose menu
    /// offers `HEAD` and the base branch the Server names, the current one checked. Without
    /// a base there is nothing to choose, so the button names `HEAD` and is disabled.
    fn render_changes_comparison(
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
        let control = match git.base.as_ref().map(|base| base.branch.clone()) {
            None => button
                .disabled(true)
                .tooltip("No base branch to compare with")
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

    /// The Changes view's body: the sections and their trees, or why there are none.
    fn render_changes_list(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        changes: &GitChanges,
        shown: Option<RelativePathBuf>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Copied out so the rows below can take `cx` again.
        let theme = cx.theme();
        let (radius, muted, warning) = (theme.radius, theme.muted_foreground, theme.warning);
        let entries = &changes.entries;
        if entries.is_empty() {
            return empty_state("No changes", cx);
        }

        let mut list = v_flex()
            .id("condr-changes-list")
            .flex_1()
            .min_h_0()
            .w_full()
            .px_1()
            .pb_1()
            .overflow_y_scrollbar();
        if changes.truncated {
            list = list.child(
                div()
                    .px_2()
                    .py_1()
                    .text_xs()
                    .text_color(warning)
                    .child(format!(
                        "Only the first {MAX_GIT_CHANGES} changes are listed; add a .gitignore for the rest."
                    )),
            );
        }
        for section in ChangesSection::ORDER {
            let rows: Vec<&GitChangeEntry> = entries
                .iter()
                .filter(|entry| ChangesSection::of(entry.status) == section)
                .collect();
            if rows.is_empty() {
                continue;
            }
            let collapsed = self
                .files_view
                .collapsed_changes_sections
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
                    .child(div().text_xs().text_color(muted).child(format!(
                        "{} ({})",
                        section.title(),
                        rows.len()
                    ))),
            );
            if collapsed {
                continue;
            }
            let folded = |path: &RelativePath| {
                self.files_view.collapsed_change_dirs.contains(&(
                    key,
                    workspace_id,
                    section,
                    path.to_relative_path_buf(),
                ))
            };
            for row in tree_rows(&rows, folded) {
                list = list.child(self.render_change_row(
                    key,
                    workspace_id,
                    section,
                    row,
                    shown.as_deref(),
                    cx,
                ));
            }
        }
        list.into_any_element()
    }

    /// A Changes row: a directory folds, a file opens in the Diff Tab and has the file menu.
    fn render_change_row(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        section: ChangesSection,
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
            } => {
                // The section is part of the id: the same directory in two sections is two
                // rows, and rows sharing an id would share one click state.
                let id = format!("{}-dir-{path}", section.selector());
                directory_row(id, label, depth, folded, cx)
                    .on_click(move |_, _, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            let dir = (key, workspace_id, section, path.clone());
                            if !this.files_view.collapsed_change_dirs.remove(&dir) {
                                this.files_view.collapsed_change_dirs.insert(dir);
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
                .context_menu(self.file_context_menu(
                    key,
                    workspace_id,
                    &entry.path,
                    Some(!deleted),
                    cx,
                ))
                .into_any_element()
            }
        }
    }

    /// The Diff Tab's body: one file's diff, or why there is none.
    pub(super) fn render_diff_tab(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: TabId,
        path: &RelativePath,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let path_text = path.to_string();
        let git = self
            .connection(key)
            .and_then(|connection| connection.model.workspace_git.get(&workspace_id));
        let against = git.map_or(DiffBase::Head, |git| self.diff_base(key, workspace_id, git));
        let commit = self.diff_commit(key, workspace_id, path);
        let entry = match commit {
            // The commit's own entry, once History has its details.
            Some(commit) => self
                .connection(key)
                .and_then(|connection| {
                    connection
                        .resources
                        .commits
                        .get(&(workspace_id, commit.id.clone()))
                })
                .and_then(|details| details.as_ref().ok())
                .and_then(|details| {
                    details
                        .changes
                        .entries
                        .iter()
                        .find(|entry| entry.path == path)
                }),
            None => git.and_then(|git| {
                shown_changes(git, against)
                    .entries
                    .iter()
                    .find(|entry| entry.path == path)
            }),
        };
        let comparison = match (commit, git) {
            (Some(commit), _) => format!("Commit {}", super::history::short_id(&commit.id)),
            (None, Some(git)) => comparison_label(git, against),
            (None, None) => "Against HEAD".to_owned(),
        };
        let viewer = self.files_view.diff_viewers.get(&(key, tab_id));
        let content = viewer.map_or(DiffContent::Loading, |viewer| viewer.content.clone());
        let header = h_flex()
            .debug_selector(|| "diff-header".into())
            .flex_none()
            .h_9()
            .w_full()
            .px_3()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .when_some(entry, |this, entry| {
                this.child(status_glyph(entry.status, cx))
            })
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_sm()
                    .font_medium()
                    .child(path_text.clone()),
            )
            .child(
                div()
                    .debug_selector(|| "diff-comparison".into())
                    .flex_none()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(comparison),
            )
            .when_some(entry.and_then(|entry| entry.stat), |this, stat| {
                this.child(diff_stat(stat, cx))
            })
            .when(content == DiffContent::Text, |this| {
                let owner = cx.weak_entity();
                let unified = self.files_view.diff_unified;
                this.child(
                    ButtonGroup::new("diff-mode")
                        .ghost()
                        .xsmall()
                        .child(
                            Button::new("diff-mode-split")
                                .debug_selector(|| "diff-mode-split".into())
                                .icon(Icon::new(gpui_kit::assets::IconName::Columns2))
                                .tooltip("Split")
                                .accessibility_label("Split")
                                .selected(!unified),
                        )
                        .child(
                            Button::new("diff-mode-unified")
                                .debug_selector(|| "diff-mode-unified".into())
                                .icon(Icon::new(gpui_kit::assets::IconName::Rows2))
                                .tooltip("Unified")
                                .accessibility_label("Unified")
                                .selected(unified),
                        )
                        .on_click(move |selected, _, cx| {
                            let unified = selected.first() == Some(&1);
                            let _ = owner.update(cx, |this, cx| this.set_diff_unified(unified, cx));
                        }),
                )
            })
            .child({
                // A deleted file has nothing on disk to show.
                let deleted = entry.is_some_and(|entry| entry.status == GitChangeStatus::Deleted);
                let owner = cx.weak_entity();
                let path = path.to_relative_path_buf();
                Button::new("diff-show-file")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(CondrIconName::FileText))
                    .debug_selector(|| "diff-show-file".into())
                    .tooltip("Show File")
                    .accessibility_label("Show File")
                    .disabled(deleted)
                    .on_click(move |_, window, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            this.show_file_from_diff(
                                key,
                                workspace_id,
                                tab_id,
                                path.clone(),
                                window,
                                cx,
                            );
                        });
                    })
            });
        let body = match (&content, viewer) {
            (DiffContent::Text, Some(viewer)) => {
                let owner = cx.weak_entity();
                div()
                    .debug_selector(|| "diff-body".into())
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        // The Tab header already names the file, its status and its counts.
                        // Two columns wrap: Kit sizes each to the longest line, which would
                        // push the new side out of view.
                        Diff::new(&viewer.state)
                            .header_visible(false)
                            .soft_wrap(!self.files_view.diff_unified)
                            .on_line_click(move |position, _, _, cx| {
                                let position = position.clone();
                                let _ = owner.update(cx, |this, _| {
                                    if let Some(viewer) =
                                        this.files_view.diff_viewers.get_mut(&(key, tab_id))
                                    {
                                        viewer.clicked = Some(position);
                                    }
                                });
                            })
                            .size_full(),
                    )
                    .into_any_element()
            }
            (DiffContent::Loading, _) => empty_state("Loading diff…", cx),
            (DiffContent::Unchanged, _) => empty_state(format!("No changes in {path_text}"), cx),
            (DiffContent::Binary, _) => empty_state("Binary file", cx),
            (DiffContent::TooLarge { bytes }, _) => empty_state(
                format!(
                    "Diff too large to show ({:.1} MiB)",
                    *bytes as f64 / (1024. * 1024.)
                ),
                cx,
            ),
            (DiffContent::Failed(reason), _) => div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .text_sm()
                .text_color(theme.danger)
                .child(reason.clone())
                .into_any_element(),
            (DiffContent::Text, None) => empty_state("Loading diff…", cx),
        };
        v_flex()
            .debug_selector(|| "diff-tab".into())
            .size_full()
            .child(header)
            .when_some(commit.cloned(), |this, commit| {
                this.child(self.render_commit_header(key, workspace_id, &commit, cx))
            })
            .child(body)
            .into_any_element()
    }

    /// The Diff Tab's "Show File": opens the file in the Preview Tab at the line last
    /// clicked in the diff, or at the first change (ADR 0017, ADR 0046). The line is this
    /// Client's presentation: it waits in `pending_file_line` until the text lands.
    fn show_file_from_diff(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: TabId,
        path: RelativePathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A commit's line numbers say nothing about today's file, so it opens at its top.
        if self.diff_commit(key, workspace_id, &path).is_some() {
            self.files_view.pending_file_line = None;
            self.show_file_on(key, workspace_id, path, window, cx);
            return;
        }
        let clicked = self
            .files_view
            .diff_viewers
            .get(&(key, tab_id))
            .and_then(|viewer| viewer.clicked.clone());
        let line = self
            .connection(key)
            .and_then(|connection| {
                connection
                    .resources
                    .diffs
                    .get(&(workspace_id, path.clone()))
            })
            .and_then(|answer| answer.as_ref().ok())
            .and_then(|diff| match &diff.content {
                FileDiffContent::Text { hunks } => Some(hunks),
                _ => None,
            })
            .and_then(|hunks| file_line_for_click(hunks, clicked.as_ref()));
        self.files_view.pending_file_line =
            line.map(|line| (key, workspace_id, path.clone(), line));
        self.show_file_on(key, workspace_id, path, window, cx);
    }

    /// Brings the Diff Tab's `Diff` in line with the Server's answer for its file: asks for
    /// the diff once, then hands the hunks to Kit as a patch (ADR 0046). Called from
    /// `rebuild_dock`, the one place that sees every presentation change.
    pub(super) fn sync_diff_view(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        tab_id: TabId,
        path: RelativePathBuf,
        cx: &mut Context<Self>,
    ) {
        let commit = self
            .diff_commit(key, workspace_id, &path)
            .map(|commit| commit.id.clone());
        let Some(connection) = self.connection(key) else {
            return;
        };
        let resources = &connection.resources;
        // A commit's answer never changes, so it needs no generation.
        let (answer, generation, status) = match &commit {
            Some(commit) => (
                resources
                    .commit_diffs
                    .get(&(workspace_id, commit.clone(), path.clone()))
                    .cloned(),
                0,
                resources
                    .commits
                    .get(&(workspace_id, commit.clone()))
                    .and_then(|details| details.as_ref().ok())
                    .and_then(|details| {
                        details
                            .changes
                            .entries
                            .iter()
                            .find(|entry| entry.path == path)
                    })
                    .map(|entry| entry.status),
            ),
            None => (
                resources.diffs.get(&(workspace_id, path.clone())).cloned(),
                resources.diffs_generation,
                connection
                    .model
                    .workspace_git
                    .get(&workspace_id)
                    .and_then(|git| {
                        shown_changes(git, self.diff_base(key, workspace_id, git))
                            .entries
                            .iter()
                            .find(|entry| entry.path == path)
                            .map(|entry| entry.status)
                    }),
            ),
        };
        let request = answer.is_none()
            && commit.is_none()
            && !resources
                .pending_diffs
                .contains_key(&(workspace_id, path.clone()));
        match &commit {
            Some(commit) if answer.is_none() => {
                self.request_commit_diff(key, workspace_id, commit.clone(), path.clone());
            }
            _ if request => self.request_diff(key, workspace_id, path.clone()),
            _ => {}
        }

        let mode = diff_mode(self.files_view.diff_unified);
        let viewer = self
            .files_view
            .diff_viewers
            .entry((key, tab_id))
            .or_insert_with(|| DiffViewer {
                state: cx.new(|cx| DiffState::new([], cx).with_mode(mode)),
                shown: None,
                patch: String::new(),
                content: DiffContent::Loading,
                clicked: None,
            });
        let source = (path, commit);
        let Some(answer) = answer else {
            if viewer
                .shown
                .as_ref()
                .is_none_or(|(shown, _)| *shown != source)
            {
                viewer.content = DiffContent::Loading;
                viewer.shown = None;
            }
            return;
        };
        if viewer.shown.as_ref() == Some(&(source.clone(), generation)) {
            return;
        }
        let retargeted = viewer
            .shown
            .as_ref()
            .is_none_or(|(shown, _)| *shown != source);
        viewer.shown = Some((source, generation));
        if retargeted {
            viewer.clicked = None;
        }
        let patch = match answer {
            Err(reason) => Err(DiffContent::Failed(reason)),
            Ok(diff) => match diff.content {
                FileDiffContent::Binary => Err(DiffContent::Binary),
                FileDiffContent::TooLarge { bytes } => Err(DiffContent::TooLarge { bytes }),
                FileDiffContent::Text { hunks } if hunks.is_empty() => Err(DiffContent::Unchanged),
                FileDiffContent::Text { hunks } => Ok(patch_text(status, &hunks)),
            },
        };
        let patch = match patch {
            Ok(patch) => patch,
            Err(content) => {
                viewer.content = content;
                viewer.patch.clear();
                return;
            }
        };
        if patch == viewer.patch {
            viewer.content = DiffContent::Text;
            return;
        }
        match DiffFile::parse(&patch) {
            Ok(files) => {
                let clicked = viewer.clicked.clone();
                viewer.state.update(cx, |state, cx| {
                    state.set_files(files, cx);
                    // ponytail: Kit keeps no viewport across `set_files`, so a changed
                    // diff returns to the line last clicked, else to the top; keep the
                    // top line once Kit can say which it is.
                    if let Some(line) = clicked {
                        state.scroll_to_line(line, cx);
                    }
                });
                viewer.content = DiffContent::Text;
                viewer.patch = patch;
            }
            Err(error) => {
                viewer.content = DiffContent::Failed(format!("Could not show this diff: {error}"));
                viewer.patch.clear();
            }
        }
    }

    fn set_diff_unified(&mut self, unified: bool, cx: &mut Context<Self>) {
        self.files_view.diff_unified = unified;
        let mode = diff_mode(unified);
        for viewer in self.files_view.diff_viewers.values() {
            viewer
                .state
                .update(cx, |state, cx| state.set_mode(mode, cx));
        }
        self.schedule_state_save(cx);
        cx.notify();
    }

    fn request_diff(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        path: RelativePathBuf,
    ) {
        let against = self
            .connection(key)
            .and_then(|connection| connection.model.workspace_git.get(&workspace_id))
            .map_or(DiffBase::Head, |git| self.diff_base(key, workspace_id, git));
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
        if connection.send(ClientMessage::GitDiff {
            server_id,
            session_id,
            request_id,
            workspace_id,
            path: path.clone(),
            against,
            full_context: true,
            commit: None,
        }) {
            connection
                .resources
                .pending_diffs
                .insert((workspace_id, path), request_id);
        }
    }
}

/// The change list a comparison shows; `HEAD`'s when the Server reports no base.
fn shown_changes(git: &WorkspaceGitSnapshot, against: DiffBase) -> &GitChanges {
    match (against, &git.base) {
        (DiffBase::MergeBase, Some(base)) => &base.changes,
        _ => &git.changes,
    }
}

/// What the Changes header and the Diff Tab header say a comparison is against.
fn comparison_label(git: &WorkspaceGitSnapshot, against: DiffBase) -> String {
    match (against, &git.base) {
        (DiffBase::MergeBase, Some(base)) => format!("Against {}", base.branch),
        _ => "Against HEAD".to_owned(),
    }
}

pub(super) fn empty_state(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .p_4()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
        .into_any_element()
}

fn diff_mode(unified: bool) -> DiffMode {
    if unified {
        DiffMode::Unified
    } else {
        DiffMode::Split
    }
}

/// The Server's hunks written as the Git patch Kit's `DiffFile::parse` reads (ADR 0046):
/// the wire carries structured lines, and Kit has no constructor that takes them.
fn patch_text(status: Option<GitChangeStatus>, hunks: &[DiffHunk]) -> String {
    let added = matches!(
        status,
        Some(GitChangeStatus::Added | GitChangeStatus::Untracked)
    );
    let old = if added {
        "/dev/null".to_owned()
    } else {
        format!("a/{PATCH_PATH}")
    };
    let new = if status == Some(GitChangeStatus::Deleted) {
        "/dev/null".to_owned()
    } else {
        format!("b/{PATCH_PATH}")
    };
    let mut text = format!("--- {old}\n+++ {new}\n");
    for hunk in hunks {
        text.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines
        ));
        for line in &hunk.lines {
            text.push(match line.kind {
                DiffLineKind::Context => ' ',
                DiffLineKind::Added => '+',
                DiffLineKind::Removed => '-',
            });
            text.push_str(&line.text);
            text.push('\n');
        }
    }
    text
}

/// The 0-based line of the new file a click in the diff is about: the line itself on the
/// new side, and for a line only the old side has, the new line it was removed before. With
/// no click, the first change. `None` when there is no line at all.
fn file_line_for_click(hunks: &[DiffHunk], clicked: Option<&DiffLinePosition>) -> Option<u32> {
    let lines = || hunks.iter().flat_map(|hunk| &hunk.lines);
    let old_number = match clicked {
        Some(position) if position.side() == DiffSide::Modified => {
            return u32::try_from(position.line()).ok().map(|line| line - 1);
        }
        Some(position) => u32::try_from(position.line()).ok(),
        None => {
            let first = lines().find(|line| line.kind != DiffLineKind::Context)?;
            if let Some(new) = first.new_number {
                return Some(new - 1);
            }
            first.old_number
        }
    }?;
    let mut after = lines().skip_while(|line| line.old_number != Some(old_number));
    let clicked = after.next()?;
    if let Some(new) = clicked.new_number {
        return Some(new - 1);
    }
    // Removed: the next line the new side has, or the end of the file.
    after
        .find_map(|line| line.new_number)
        .map(|new| new - 1)
        .or_else(|| lines().filter_map(|line| line.new_number).next_back())
        .or(Some(0))
}

#[cfg(test)]
mod tests {
    use super::{ChangesSection, PATCH_PATH, file_line_for_click, patch_text};
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

    fn hunks() -> [condr_core::DiffHunk; 2] {
        use condr_core::{DiffHunk, DiffLine, DiffLineKind};
        let line = |kind, old_number, new_number, text: &str| DiffLine {
            kind,
            old_number,
            new_number,
            text: text.to_owned(),
        };
        // @@ -1,2 +1,3 @@ / " alpha" / "-beta" / "+BETA" / "+gamma"
        // @@ -10,1 +11,1 @@ / "-old" / "+new"
        [
            DiffHunk {
                old_start: 1,
                old_lines: 2,
                new_start: 1,
                new_lines: 3,
                lines: vec![
                    line(DiffLineKind::Context, Some(1), Some(1), "alpha"),
                    line(DiffLineKind::Removed, Some(2), None, "beta"),
                    line(DiffLineKind::Added, None, Some(2), "BETA"),
                    line(DiffLineKind::Added, None, Some(3), "gamma"),
                ],
            },
            DiffHunk {
                old_start: 10,
                old_lines: 1,
                new_start: 11,
                new_lines: 1,
                lines: vec![
                    line(DiffLineKind::Removed, Some(10), None, "old"),
                    line(DiffLineKind::Added, None, Some(11), "new"),
                ],
            },
        ]
    }

    #[test]
    fn a_clicked_diff_line_names_the_new_file_line_it_is_about() {
        use gpui_kit::component::diff::{DiffLinePosition, DiffSide};
        let hunks = hunks();
        let click = |side, line| {
            file_line_for_click(&hunks, Some(&DiffLinePosition::new(PATCH_PATH, side, line)))
        };
        assert_eq!(click(DiffSide::Modified, 2), Some(1), "BETA");
        assert_eq!(
            click(DiffSide::Original, 1),
            Some(0),
            "alpha, on the old side"
        );
        assert_eq!(
            click(DiffSide::Original, 2),
            Some(1),
            "beta was removed before BETA"
        );
        assert_eq!(
            click(DiffSide::Original, 10),
            Some(10),
            "old was removed before new"
        );
        assert_eq!(
            file_line_for_click(&hunks, None),
            Some(1),
            "no click: the first change"
        );
        assert_eq!(file_line_for_click(&[], None), None);
        let emptied = [condr_core::DiffHunk {
            old_start: 1,
            old_lines: 1,
            new_start: 0,
            new_lines: 0,
            lines: vec![condr_core::DiffLine {
                kind: condr_core::DiffLineKind::Removed,
                old_number: Some(1),
                new_number: None,
                text: String::new(),
            }],
        }];
        assert_eq!(file_line_for_click(&emptied, None), Some(0));
    }

    #[test]
    fn hunks_written_as_a_patch_parse_back_with_their_status() {
        use gpui_kit::component::diff::{DiffFile, DiffFileStatus};
        let parse = |status| {
            let mut files = DiffFile::parse(&patch_text(status, &hunks())).unwrap();
            assert_eq!(files.len(), 1);
            files.remove(0)
        };
        let modified = parse(Some(GitChangeStatus::Modified));
        assert_eq!(modified.status(), DiffFileStatus::Modified);
        assert_eq!((modified.additions(), modified.deletions()), (3, 2));
        assert_eq!(modified.path().as_ref(), PATCH_PATH);

        let added = [condr_core::DiffHunk {
            old_start: 0,
            old_lines: 0,
            new_start: 1,
            new_lines: 1,
            lines: vec![condr_core::DiffLine {
                kind: condr_core::DiffLineKind::Added,
                old_number: None,
                new_number: Some(1),
                text: "new".into(),
            }],
        }];
        let file = DiffFile::parse(&patch_text(Some(GitChangeStatus::Untracked), &added)).unwrap();
        assert_eq!(file[0].status(), DiffFileStatus::Added);
        assert_eq!(parse(None).status(), DiffFileStatus::Modified);
    }
}
