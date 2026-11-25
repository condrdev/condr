//! The right sidebar (ADR 0017, ADR 0018, ADR 0046): the presented Workspace's Files,
//! Changes and History, one view at a time, remembered per Workspace (ADR 0023). Each view
//! is its own module; this one holds what they share: whether the sidebar is open, which
//! view it shows, its width, header and Refresh.

mod change_tree;
mod changes;
mod entries;
mod files;
mod history;

use super::placeholders::empty_state;
use super::*;
use condr_core::GitDiffStat;
use gpui_kit::component::tab::{Tab, TabBar};
use rust_i18n::t;

pub(super) use changes::*;
pub(super) use entries::*;
pub(super) use files::*;
pub(super) use history::*;

/// What the right sidebar shows (ADR 0018); remembered per Workspace (ADR 0023).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum RightSidebarView {
    #[default]
    Changes,
    Files,
    /// The commits reachable from `HEAD` (ADR 0046).
    History,
}

pub(super) const INITIAL_RIGHT_SIDEBAR_WIDTH: Pixels = px(320.);

pub(super) const MIN_RIGHT_SIDEBAR_WIDTH: Pixels = px(200.);

pub(super) const MAX_RIGHT_SIDEBAR_WIDTH: Pixels = px(480.);

/// Dragging the right sidebar's left edge resizes it.
pub(super) struct DraggedRightSidebar;

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
    pub(super) history: HistoryState,
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
        self.history
            .selected_commits
            .retain(|(connection, id), _| workspace_live(*connection, *id));
        self.history
            .repositories
            .retain(|(connection, id), _| workspace_live(*connection, *id));
        self.history
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

    /// A window draws the right sidebar of the Workspace it draws; a Tab window has none.
    pub(super) fn right_sidebar_open(&self, view: &WindowView) -> bool {
        !matches!(view.scope, WindowScope::Tab(..))
            && self.draws_workspace(view)
            && self
                .presented_workspace_in(view)
                .is_some_and(|workspace| self.right_sidebar.open.contains(&workspace))
    }

    /// The title bar's toggle for the right sidebar, next to "Open in". Only a Workspace
    /// has anything to show, so the welcome page renders none.
    pub(super) fn render_changes_toggle(
        &self,
        view: &WindowView,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let owner = cx.weak_entity();
        let open = self.right_sidebar_open(view);
        let label = if open {
            t!("changes.toggle.hide")
        } else {
            t!("changes.toggle.show")
        };
        let icon = if open {
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
                    .tooltip_with_action(label.clone(), &ToggleChanges, Some(SHORTCUT_CONTEXT))
                    .accessibility_label(label)
                    .on_click(move |_, _, cx| {
                        let _ = owner.update(cx, |this, cx| this.toggle_right_sidebar(cx));
                    }),
            )
            .into_any_element()
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

    fn set_right_sidebar_view(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        view: RightSidebarView,
        cx: &mut Context<Self>,
    ) {
        if self.right_sidebar.views.insert((key, workspace_id), view) != Some(view) {
            self.schedule_state_save(cx);
            cx.notify();
        }
    }

    /// The right sidebar: the presented Workspace's changes against `HEAD`, or its files
    /// (ADR 0018), chosen by the tabs in the header.
    pub(super) fn render_right_sidebar(
        &self,
        view: &WindowView,
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

        let presented =
            self.presented_in(view)
                .and_then(|(key, _, workspace_id, presented_tab)| {
                    let connection = self.connection(key)?;
                    let session = connection.session()?;
                    let workspace = session.workspace(workspace_id)?;
                    let presented_tab = Some(presented_tab);
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
                .child(self.sidebar_header(None, RightSidebarView::Changes, None, false, cx))
                .child(empty_state(t!("changes.no_workspace"), cx))
                .into_any_element();
        };
        // A Server that sends `head` serves History (ADR 0046).
        let history = git
            .as_ref()
            .and_then(|git| git.repositories.first())
            .is_some_and(|repository| repository.head.is_some());
        let view = match self.right_sidebar_view_for(key, workspace_id, git.is_some()) {
            RightSidebarView::History if !history => RightSidebarView::Changes,
            view => view,
        };
        let against = self.workspace_diff_base(key, workspace_id);
        let count = git.as_ref().map(|git| shown_entries(git, against).count());
        let total = git.as_ref().and_then(|git| {
            shown_entries(git, against)
                .filter_map(|entry| entry.stat)
                .reduce(|sum, stat| GitDiffStat {
                    added: sum.added + stat.added,
                    deleted: sum.deleted + stat.deleted,
                })
        });
        let header = self.sidebar_header(Some((key, workspace_id)), view, count, history, cx);
        let body = match view {
            RightSidebarView::Files => {
                self.render_files_list(key, workspace_id, shown_file.as_deref(), cx)
            }
            RightSidebarView::Changes => match &git {
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
                        git,
                        against,
                        shown_diff,
                        cx,
                    ))
                    .into_any_element(),
                None => empty_state(
                    t!("changes.not_a_repository", workspace = workspace_name),
                    cx,
                ),
            },
            RightSidebarView::History => v_flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .children(
                    git.as_ref()
                        .map(|git| self.render_history_head(key, workspace_id, git, cx)),
                )
                .child(self.render_history(key, workspace_id, shown_diff.as_deref(), cx))
                .into_any_element(),
        };
        column
            .child(header)
            .child(body)
            .children(self.render_transfer_strip(key, workspace_id, cx))
            .into_any_element()
    }

    /// The Files / Changes / History tabs, sized to their labels, with the change count in
    /// the Changes tab and Refresh at the far end whichever view is shown.
    fn sidebar_header(
        &self,
        target: Option<(ConnectionKey, WorkspaceId)>,
        view: RightSidebarView,
        count: Option<usize>,
        history: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // GPUI Kit's own tab strip, as the Kit gallery shows it; the count rides in the
        // Changes tab and Refresh takes the strip's free end.
        let owner = cx.weak_entity();
        let selected = match view {
            RightSidebarView::Files => 0,
            RightSidebarView::Changes => 1,
            RightSidebarView::History => 2,
        };
        let refresh = target.map(|(key, workspace_id)| {
            let owner = cx.weak_entity();
            Button::new("sidebar-refresh")
                .ghost()
                .xsmall()
                .icon(Icon::new(IconName::RefreshCw))
                .debug_selector(|| "sidebar-refresh".into())
                .tooltip(t!("changes.refresh"))
                .accessibility_label(t!("changes.refresh"))
                .loading(
                    self.right_sidebar
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
                    1 => RightSidebarView::Changes,
                    2 => RightSidebarView::History,
                    _ => RightSidebarView::Files,
                };
                let _ = owner.update(cx, |this, cx| {
                    if let Some((key, workspace_id)) = target {
                        this.set_right_sidebar_view(key, workspace_id, view, cx);
                    }
                });
            })
            .children([
                Tab::new()
                    .label(t!("changes.tab.files"))
                    .debug_selector(|| "sidebar-view-files".into()),
                Tab::new()
                    .label(match count {
                        Some(count) => t!("changes.tab.changes_count", count = count),
                        None => t!("changes.tab.changes"),
                    })
                    .debug_selector(|| "sidebar-view-changes".into()),
            ])
            .when(history, |this| {
                this.child(
                    Tab::new()
                        .label(t!("changes.tab.history"))
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
                this.right_sidebar.refreshing.remove(&(key, workspace_id));
                cx.notify();
            });
        });
        self.right_sidebar
            .refreshing
            .insert((key, workspace_id), timeout);
        self.reload_history(key, workspace_id);
        cx.notify();
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
