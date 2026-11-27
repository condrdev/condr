//! The right sidebar (ADR 0017, ADR 0018, ADR 0046): the presented Workspace's Files,
//! Changes and Git History, one view at a time, remembered per Workspace (ADR 0023). Each view
//! is its own module; this one holds what they share: whether the sidebar is open, which
//! view it shows, its width, header and Refresh.

mod change_tree;
mod changes;
mod entries;
mod files;
mod git_history;

use super::icons::CondrIconName;
use super::placeholders::empty_state;
use super::*;
use condr_core::GitDiffStat;
use gpui_kit::component::scroll::{Scrollbar, ScrollbarMode};
use rust_i18n::t;
use std::borrow::Cow;

pub(super) use change_tree::{TreeRow, directory_row, file_row, tree_rows};
pub(super) use changes::*;
pub(super) use entries::*;
pub(super) use files::*;
pub(super) use git_history::*;

/// What the right sidebar shows, one at a time, each opened by its button in the strip
/// (ADR 0076); remembered per Workspace (ADR 0023).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum RightSidebarView {
    #[default]
    Changes,
    Files,
    /// The commits reachable from `HEAD` (ADR 0046).
    GitHistory,
}

pub(super) const INITIAL_RIGHT_SIDEBAR_WIDTH: Pixels = px(320.);

pub(super) const MIN_RIGHT_SIDEBAR_WIDTH: Pixels = px(200.);

pub(super) const MAX_RIGHT_SIDEBAR_WIDTH: Pixels = px(480.);

impl RightSidebarView {
    /// Its button in the strip: the id tests find it by, its icon and its name.
    fn button(self) -> (&'static str, Icon, Cow<'static, str>) {
        match self {
            Self::Files => (
                "right-sidebar-files",
                Icon::new(Glyph::FolderTree),
                t!("right_sidebar.files"),
            ),
            Self::Changes => (
                "right-sidebar-changes",
                Icon::new(Glyph::GitCommitHorizontal),
                t!("right_sidebar.changes"),
            ),
            Self::GitHistory => (
                "right-sidebar-git-history",
                Icon::new(CondrIconName::GitBranch),
                t!("right_sidebar.git_history"),
            ),
        }
    }
}

/// The strip of view buttons along the window's right edge (ADR 0076): a medium button and
/// the gutter either side of it.
pub(super) const RIGHT_SIDEBAR_STRIP_WIDTH: Pixels = px(40.);

/// Dragging the right sidebar's left edge resizes it.
pub(super) struct DraggedRightSidebar;

/// What a window shows of its Workspace's right sidebar.
pub(super) struct ShownRightSidebar {
    pub(super) key: ConnectionKey,
    pub(super) workspace_id: WorkspaceId,
    /// The view the sidebar shows, or opens on while it is closed.
    pub(super) view: RightSidebarView,
    pub(super) open: bool,
    /// The Server serves Git History for this Workspace (ADR 0046).
    pub(super) git_history: bool,
}

/// This Client's right sidebar per Workspace, retained against the authoritative
/// structure.
#[derive(Default)]
pub(super) struct RightSidebarState {
    /// Workspaces showing the right sidebar (ADR 0017); remembered in the GUI state
    /// file alongside `sidebar_workspace_open` (ADR 0023).
    pub(super) open: HashSet<(ConnectionKey, WorkspaceId)>,
    /// Which view the right sidebar shows per Workspace (ADR 0018), remembered in the
    /// GUI state file. Others open on Changes, or Files outside a repository.
    pub(super) views: HashMap<(ConnectionKey, WorkspaceId), RightSidebarView>,
    /// Refreshes the user asked for and the Server has not answered yet (ADR 0046), each
    /// with the timer that gives up on it.
    pub(super) refreshing: HashMap<(ConnectionKey, WorkspaceId), Task<()>>,
    /// The terminal Tab each Workspace presented last: where "Insert Path into Terminal"
    /// sends its text once a viewer Tab has taken the Workspace's active slot.
    pub(super) last_terminal_tabs: HashMap<(ConnectionKey, WorkspaceId), TabId>,
    /// Creates, renames and deletes in flight, by connection and request.
    pub(super) entry_requests: HashMap<(ConnectionKey, u64), EntryRequest>,
    pub(super) changes: ChangesState,
    pub(super) files: FilesTreeState,
    pub(super) git_history: GitHistoryState,
}

impl RightSidebarState {
    /// Forgets what `key`'s closed Workspaces and Tabs left behind; `None` is a connection
    /// gone, which leaves nothing of `key`.
    pub(super) fn retain_session(&mut self, key: ConnectionKey, session: Option<&Session>) {
        let workspace_live = |connection_key: ConnectionKey, id: WorkspaceId| {
            connection_key != key || session.is_some_and(|session| session.workspace(id).is_some())
        };
        self.open
            .retain(|(connection, id)| workspace_live(*connection, *id));
        self.views
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
        self.changes
            .against_base
            .retain(|(connection, id)| workspace_live(*connection, *id));
        self.changes
            .collapsed_dirs
            .retain(|(connection, id, _, _)| workspace_live(*connection, *id));
        self.files
            .expanded_dirs
            .retain(|(connection, id, _)| workspace_live(*connection, *id));
        self.git_history
            .selected_commits
            .retain(|(connection, id), _| workspace_live(*connection, *id));
        self.git_history
            .repositories
            .retain(|(connection, id), _| workspace_live(*connection, *id));
        self.git_history
            .collapsed_dirs
            .retain(|(connection, id, _)| workspace_live(*connection, *id));
    }
}

impl Condr {
    /// Each Workspace keeps its own right sidebar: opening it here leaves the others
    /// closed, and coming back finds it the way it was left.
    pub(super) fn toggle_right_sidebar(&mut self, cx: &mut Context<Self>) {
        let Some(workspace) = self.presented_workspace() else {
            return;
        };
        if !self.draws_workspace(self.active_view()) {
            return;
        }
        if !self.right_sidebar.open.remove(&workspace) {
            self.right_sidebar.open.insert(workspace);
        }
        self.schedule_state_save(cx);
        cx.notify();
    }

    /// What `view`'s window shows of the right sidebar. A window draws the right sidebar of
    /// the Workspace page it draws; a Tab window, Tasks and the other pages have none.
    pub(super) fn shown_right_sidebar(&self, view: &WindowView) -> Option<ShownRightSidebar> {
        if view.page != ContentPage::Workspace
            || matches!(view.scope, WindowScope::Tab(..) | WindowScope::Tasks)
            || !self.draws_workspace(view)
        {
            return None;
        }
        let (key, workspace_id) = self.presented_workspace_in(view)?;
        let git = self.connection(key)?.model.workspace_git.get(&workspace_id);
        // A Server that sends `head` serves Git History (ADR 0046).
        let git_history = git
            .and_then(|git| git.repositories.first())
            .is_some_and(|repository| repository.head.is_some());
        let shown = match self.right_sidebar_view_for(key, workspace_id, git.is_some()) {
            RightSidebarView::GitHistory if !git_history => RightSidebarView::Changes,
            shown => shown,
        };
        Some(ShownRightSidebar {
            key,
            workspace_id,
            view: shown,
            open: self.right_sidebar.open.contains(&(key, workspace_id)),
            git_history,
        })
    }

    /// The user's pick for this Workspace; a Workspace outside any repository opens on
    /// Files, since its Changes view has nothing to show (ADR 0018).
    fn right_sidebar_view_for(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        in_repository: bool,
    ) -> RightSidebarView {
        self.right_sidebar
            .views
            .get(&(key, workspace_id))
            .copied()
            .unwrap_or(if in_repository {
                RightSidebarView::Changes
            } else {
                RightSidebarView::Files
            })
    }

    /// A button of the strip: opens the sidebar on its view, or closes the sidebar when
    /// that view is the one `selected` says it shows.
    fn click_right_sidebar_view(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        view: RightSidebarView,
        selected: Option<RightSidebarView>,
        cx: &mut Context<Self>,
    ) {
        let workspace = (key, workspace_id);
        if selected == Some(view) {
            self.right_sidebar.open.remove(&workspace);
        } else {
            self.right_sidebar.open.insert(workspace);
            self.right_sidebar.views.insert(workspace, view);
        }
        self.schedule_state_save(cx);
        cx.notify();
    }

    /// The strip along the window's right edge (ADR 0076): a button per view, the one shown
    /// selected while the sidebar is open. Git History has a button only where the Server
    /// serves it.
    pub(super) fn render_right_sidebar_strip(
        &self,
        shown: &ShownRightSidebar,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let owner = cx.weak_entity();
        let (key, workspace_id) = (shown.key, shown.workspace_id);
        let selected = shown.open.then_some(shown.view);
        let button = |view: RightSidebarView| {
            let (id, icon, label) = view.button();
            let owner = owner.clone();
            Button::new(id)
                .debug_selector(move || id.into())
                .ghost()
                .icon(icon)
                .selected(selected == Some(view))
                .tooltip(label.clone())
                .accessibility_label(label)
                .on_click(move |_, _, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        this.click_right_sidebar_view(key, workspace_id, view, selected, cx);
                    });
                })
        };
        v_flex()
            .debug_selector(|| "right-sidebar-strip".into())
            .flex_none()
            .w(RIGHT_SIDEBAR_STRIP_WIDTH)
            .h_full()
            .items_center()
            .gap_1()
            // The first button sits where a title bar centres its own: on macOS the strip
            // reaches the window's top, beside the title bar.
            .pt((WORKSPACE_TITLE_BAR_HEIGHT - px(32.)) / 2.)
            .bg(theme.sidebar)
            .border_l_1()
            .border_color(theme.sidebar_border)
            .child(button(RightSidebarView::Files))
            .child(button(RightSidebarView::Changes))
            .when(shown.git_history, |this| {
                this.child(button(RightSidebarView::GitHistory))
            })
            .into_any_element()
    }

    /// The open sidebar's header, in the title bar above it (ADR 0076): the view's name,
    /// Changes with its count, and Refresh at the far end.
    pub(super) fn render_right_sidebar_header(
        &self,
        shown: &ShownRightSidebar,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (key, workspace_id) = (shown.key, shown.workspace_id);
        let title = match shown.view {
            RightSidebarView::Changes => self
                .connection(key)
                .and_then(|connection| connection.model.workspace_git.get(&workspace_id))
                .map_or_else(
                    || t!("right_sidebar.changes"),
                    |git| {
                        let against = self.workspace_diff_base(key, workspace_id);
                        let count = shown_entries(git, against).count();
                        t!("right_sidebar.changes_count", count = count)
                    },
                ),
            view => view.button().2,
        };
        let owner = cx.weak_entity();
        let theme = cx.theme();
        h_flex()
            .debug_selector(|| "right-sidebar-header".into())
            .flex_1()
            .min_w_0()
            .h_full()
            .pl_3()
            .pr_2()
            .gap_2()
            .items_center()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_l_1()
            .border_color(theme.sidebar_border)
            .child(div().flex_1().min_w_0().truncate().child(title))
            .when(shown.view == RightSidebarView::GitHistory, |this| {
                let owner = owner.clone();
                this.child(
                    Button::new("right-sidebar-git-graph")
                        .debug_selector(|| "right-sidebar-git-graph".into())
                        .ghost()
                        .small()
                        .icon(Icon::new(Glyph::GitGraph))
                        .tooltip(t!("right_sidebar.open_git_graph"))
                        .accessibility_label(t!("right_sidebar.open_git_graph"))
                        // Inside the title bar an unclaimed press starts a window move.
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(move |_, window, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                this.open_git_graph(key, workspace_id, window, cx);
                            });
                        }),
                )
            })
            .child(
                Button::new("right-sidebar-refresh")
                    .debug_selector(|| "right-sidebar-refresh".into())
                    .ghost()
                    .small()
                    .icon(Icon::new(IconName::RefreshCw))
                    .tooltip(t!("right_sidebar.refresh"))
                    .accessibility_label(t!("right_sidebar.refresh"))
                    .loading(
                        self.right_sidebar
                            .refreshing
                            .contains_key(&(key, workspace_id)),
                    )
                    // Inside the title bar an unclaimed press starts a window move on Windows.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, _, cx| {
                        let _ = owner.update(cx, |this, cx| {
                            this.refresh_workspace(key, workspace_id, cx);
                        });
                    }),
            )
            .into_any_element()
    }

    /// The open sidebar's body below its header: the view `shown` names, and the transfers
    /// in flight at its foot.
    pub(super) fn render_right_sidebar(
        &self,
        view: &WindowView,
        shown: &ShownRightSidebar,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let column = v_flex()
            .debug_selector(|| "condr-right-sidebar".into())
            .size_full()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_l_1()
            .border_color(theme.sidebar_border);
        let (key, workspace_id) = (shown.key, shown.workspace_id);
        let Some((session, presented_tab)) = self
            .presented_in(view)
            .map(|(_, session, _, presented_tab)| (session, Some(presented_tab)))
        else {
            return column.into_any_element();
        };
        let Some(workspace) = session.workspace(workspace_id) else {
            return column.into_any_element();
        };
        let shown_diff = session
            .diff_tab(workspace_id)
            .filter(|tab| presented_tab == Some(tab.id()))
            .and_then(|tab| tab.diff().map(|diff| diff.path().to_relative_path_buf()));
        let shown_file = session
            .file_tab(workspace_id)
            .filter(|tab| presented_tab == Some(tab.id()))
            .and_then(|tab| tab.file().map(|file| file.path().to_relative_path_buf()));
        let git = self
            .connection(key)
            .and_then(|connection| connection.model.workspace_git.get(&workspace_id));
        let body = match shown.view {
            RightSidebarView::Files => {
                self.render_files_list(key, workspace_id, shown_file.as_deref(), cx)
            }
            RightSidebarView::Changes => match git {
                Some(git) => {
                    let against = self.workspace_diff_base(key, workspace_id);
                    let total = shown_entries(git, against)
                        .filter_map(|entry| entry.stat)
                        .reduce(|sum, stat| GitDiffStat {
                            added: sum.added + stat.added,
                            deleted: sum.deleted + stat.deleted,
                        });
                    v_flex()
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
                            git,
                            against,
                            shown_diff,
                            cx,
                        ))
                        .into_any_element()
                }
                None => empty_state(
                    t!("changes.not_a_repository", workspace = workspace.name()),
                    cx,
                ),
            },
            RightSidebarView::GitHistory => v_flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .children(git.map(|git| self.render_git_history_head(key, workspace_id, git, cx)))
                .child(self.render_git_history(key, workspace_id, shown_diff.as_deref(), cx))
                .into_any_element(),
        };
        column
            .child(body)
            .children(self.render_transfer_strip(key, workspace_id, cx))
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
                this.right_sidebar.refreshing.remove(&(key, workspace_id));
                cx.notify();
            });
        });
        self.right_sidebar
            .refreshing
            .insert((key, workspace_id), timeout);
        self.reload_git_history(key, workspace_id);
        self.reload_git_graph(key, workspace_id);
        cx.notify();
    }
}

/// A list of the right sidebar, scrolled with its scrollbar always shown: a list longer
/// than the sidebar says so before anyone scrolls it. Kit's `overflow_y_scrollbar` follows
/// the theme's mode, which on macOS hides the bar until a scroll. Its own styles size it
/// in its parent; `content` is what scrolls.
#[derive(IntoElement)]
pub(super) struct SidebarList {
    id: ElementId,
    root: Stateful<Div>,
    content: Div,
}

pub(super) fn sidebar_list(id: impl Into<ElementId>, content: Div) -> SidebarList {
    let id = id.into();
    SidebarList {
        root: div().id(id.clone()),
        id,
        content,
    }
}

impl Styled for SidebarList {
    fn style(&mut self) -> &mut StyleRefinement {
        self.root.style()
    }
}

impl RenderOnce for SidebarList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let handle = window
            .use_keyed_state(self.id.clone(), cx, |_, _| ScrollHandle::default())
            .read(cx)
            .clone();
        self.root
            .relative()
            .overflow_hidden()
            .child(
                div()
                    .id((self.id, "area"))
                    .size_full()
                    .flex()
                    .flex_col()
                    .overflow_y_scroll()
                    .track_scroll(&handle)
                    .child(self.content.flex_none().min_h_full()),
            )
            .child(
                div().absolute().inset_0().child(
                    Scrollbar::vertical(&handle)
                        .mode(ScrollbarMode::Always)
                        .viewport_from_layout(),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::RightSidebarState;
    use condr_core::Session;

    #[test]
    fn closing_workspace_clears_its_view_without_touching_another_device() {
        let mut session = Session::new();
        session.create_workspace(std::env::temp_dir()).unwrap();
        let workspace = &session.workspaces()[0];
        let id = workspace.id();
        let tab = workspace.tabs()[0].id();
        let mut state = RightSidebarState::default();
        for key in [1, 2] {
            state.open.insert((key, id));
            state.changes.against_base.insert((key, id));
            state.files.expanded_dirs.insert((key, id, "src".into()));
            state.last_terminal_tabs.insert((key, id), tab);
        }
        state.retain_session(1, Some(&session));
        assert!(
            state.open.contains(&(1, id)),
            "reconnect keeps live view choices"
        );
        session.close_workspace(id).unwrap();
        state.retain_session(1, Some(&session));
        assert!(!state.open.contains(&(1, id)));
        assert!(!state.last_terminal_tabs.contains_key(&(1, id)));
        assert!(state.open.contains(&(2, id)));
        assert!(state.last_terminal_tabs.contains_key(&(2, id)));
        state.retain_session(2, None);
        assert!(state.open.is_empty());
        assert!(state.changes.against_base.is_empty());
        assert!(state.files.expanded_dirs.is_empty());
        assert!(state.last_terminal_tabs.is_empty());
    }
}
