//! Git Graph (ADR 0079): a Workspace's commits on every branch as IntelliJ's log shows
//! them, in a viewer Pane: the branch tree, the commit table with its graph and
//! filters, and the selected commit's files and details. It only reads.

mod branches;
mod details;
mod lanes;
mod table;

use super::right_sidebar::sidebar_list;
use super::*;
use crate::app::git_marks::short_id;
use crate::app::list_row::{RowKind, tree_row};
use crate::app::placeholders::{empty_state, failed_state};
use branches::{BranchRow, RefKind, branch_rows, short_name};
use condr_core::{GIT_LOG_PAGE, GitRef, LogStart, Viewer};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::resizable::{h_resizable, resizable_panel};
use gpui_kit::component::table::{DataTable, TableEvent, TableState};
use lanes::{GraphCommit, head_refs, layout};
use rust_i18n::t;
use table::{CommitRow, CommitTable, ref_labels};

actions!(
    git_graph,
    [
        GoToParentCommit,
        GoToChildCommit,
        FocusGitGraphFilter,
        ShowGitGraphDiff
    ]
);

/// The key context around a Git Graph Pane.
const CONTEXT: &str = "GitGraph";

/// IntelliJ's keys in its log (ADR 0079): Right to the parent, Left to the child, and its
/// shortcuts to the text filter and to the selected file's diff.
pub(super) fn key_bindings() -> [KeyBinding; 4] {
    let table = Some("GitGraph > DataTable");
    [
        KeyBinding::new("right", GoToParentCommit, table),
        KeyBinding::new("left", GoToChildCommit, table),
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-l"
            } else {
                "ctrl-l"
            },
            FocusGitGraphFilter,
            Some(CONTEXT),
        ),
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-d"
            } else {
                "ctrl-d"
            },
            ShowGitGraphDiff,
            Some(CONTEXT),
        ),
    ]
}

/// Each Git Graph Pane's view, which is this Client's own and never remembered.
#[derive(Default)]
pub(in crate::app) struct GitGraphState {
    views: HashMap<(ConnectionKey, PaneId), GitGraphView>,
}

struct GitGraphView {
    workspace_id: WorkspaceId,
    table: Entity<TableState<CommitTable>>,
    text: Entity<InputState>,
    search: Entity<InputState>,
    /// The ref the commits are filtered to, by its full name or `HEAD`; `None` for every ref.
    branch: Option<String>,
    /// The text filter as last applied, on Enter or when its field lost the keyboard.
    text_filter: String,
    selected_ref: Option<String>,
    folded: HashSet<String>,
    selected_commit: Option<String>,
    selected_file: Option<RelativePathBuf>,
    collapsed_dirs: HashSet<RelativePathBuf>,
    /// The details list every branch that reaches the commit, not only the first six.
    all_branches: bool,
    _subscriptions: Vec<Subscription>,
}

impl GitGraphState {
    /// Forgets the views of `key`'s Git Graph Panes that closed; `None` is a connection gone.
    pub(in crate::app) fn retain_session(&mut self, key: ConnectionKey, session: Option<&Session>) {
        self.views.retain(|(connection, pane_id), _| {
            *connection != key || viewer_is(session, *pane_id, ViewerKind::GitGraph)
        });
    }
}

/// The checked-out branch's short name in `repository`.
fn current_branch(repository: &RepositoryGitSnapshot) -> Option<&str> {
    repository.branch.as_deref()
}

/// The rows a log draws: each commit with its slice of the graph and its labels.
fn graph_rows(log: &GitLog, current: Option<&str>) -> Vec<CommitRow> {
    let heads = head_refs(
        log.refs
            .iter()
            .map(|reference| (reference.name.as_str(), reference.commit.as_str())),
    );
    let commits: Vec<GraphCommit<'_>> = log
        .commits
        .iter()
        .map(|commit| GraphCommit {
            id: &commit.id,
            parents: &commit.parents,
            head_ref: heads.get(commit.id.as_str()).copied(),
        })
        .collect();
    let mut labels = ref_labels(&log.refs, current);
    log.commits
        .iter()
        .zip(layout(&commits))
        .map(|(commit, graph)| {
            let (labels, tagged) = labels.remove(&commit.id).unwrap_or_default();
            CommitRow {
                commit: commit.clone(),
                graph,
                labels,
                tagged,
            }
        })
        .collect()
}

impl Condr {
    /// Whether the Server serves Git History for this Workspace, and so Git Graph (ADR 0046).
    pub(in crate::app) fn serves_git_history(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
    ) -> bool {
        self.connection(key)
            .and_then(|connection| connection.model.workspace_git.get(&workspace_id))
            .and_then(|git| git.repositories.first())
            .is_some_and(|repository| repository.head.is_some())
    }

    /// Opens the Workspace's Git Graph Pane, creating it the first time (ADR 0078, ADR 0079).
    pub(in crate::app) fn open_git_graph(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send_presenting_layout_to(
            key,
            LayoutCommand::ShowViewer {
                workspace_id,
                viewer: Viewer::GitGraph,
            },
            window,
            cx,
        );
    }

    /// The window action: the presented Workspace's Git Graph, where it has Git History.
    pub(in crate::app) fn open_git_graph_action(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((key, workspace_id)) = self.presented_workspace() else {
            return;
        };
        if self.serves_git_history(key, workspace_id) {
            self.open_git_graph(key, workspace_id, window, cx);
        }
    }

    /// Makes the view of a Git Graph Pane this window shows, the first time it shows: its
    /// table, its two fields and what listens to them.
    pub(in crate::app) fn sync_git_graph_view(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.git_graph.views.contains_key(&(key, pane_id)) {
            return;
        }
        let owner = cx.weak_entity();
        let table = cx.new(|cx| {
            TableState::new(
                CommitTable {
                    owner,
                    key,
                    pane_id,
                    rows: Vec::new(),
                    more: false,
                    loading: true,
                    user_email: None,
                    tint_head: true,
                    filtered: false,
                    generation: None,
                    width: px(0.),
                },
                window,
                cx,
            )
            .col_movable(false)
            .sortable(false)
        });
        let text =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("git_graph.text_placeholder")));
        let search = cx
            .new(|cx| InputState::new(window, cx).placeholder(t!("git_graph.branch_placeholder")));
        let subscriptions = vec![
            cx.subscribe_in(&table, window, move |this, _, event: &TableEvent, _, cx| {
                if let TableEvent::SelectRow(row_ix) = event {
                    this.select_git_graph_row(key, pane_id, *row_ix, cx);
                }
            }),
            cx.subscribe(&text, move |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                    this.apply_git_graph_text(key, pane_id, cx);
                }
            }),
            cx.subscribe(&search, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        self.git_graph.views.insert(
            (key, pane_id),
            GitGraphView {
                workspace_id,
                table,
                text,
                search,
                branch: None,
                text_filter: String::new(),
                selected_ref: None,
                folded: HashSet::new(),
                selected_commit: None,
                selected_file: None,
                collapsed_dirs: HashSet::new(),
                all_branches: false,
                _subscriptions: subscriptions,
            },
        );
    }

    /// Asks for the first page of a Git Graph Pane, or with `more` for the next; the shown
    /// list stays until the answer lands.
    pub(in crate::app) fn request_git_graph_page(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        more: bool,
    ) {
        let (Some(view), Some(connection)) = (
            self.git_graph.views.get(&(key, pane_id)),
            self.connection(key),
        ) else {
            return;
        };
        let resources = &connection.resources;
        let (from, skip) = if more {
            let Some(Ok(log)) = resources.graph_logs.get(&pane_id) else {
                return;
            };
            if !log.more || resources.pending_graph_logs.contains_key(&pane_id) {
                return;
            }
            (log.from.clone(), log.commits.len() as u32)
        } else {
            (Vec::new(), 0)
        };
        // `HEAD` too as a ref: in date order, with the refs (ADR 0079).
        let start = match view.branch.as_deref() {
            None => LogStart::All,
            Some(name) => LogStart::Ref(name.to_owned()),
        };
        let text = (!view.text_filter.is_empty()).then(|| view.text_filter.clone());
        let workspace_id = view.workspace_id;
        let repository = self.git_history_directory(key, workspace_id);
        if let Some(request_id) = self.send_request(key, |server_id, session_id, request_id| {
            ClientMessage::GitLog {
                server_id,
                session_id,
                request_id,
                workspace_id,
                from,
                start,
                text,
                skip,
                limit: GIT_LOG_PAGE,
                repository,
            }
        }) && let Some(connection) = self.connection_mut(key)
        {
            connection
                .resources
                .pending_graph_logs
                .insert(pane_id, (request_id, skip));
        }
    }

    /// The first page again for every Git Graph Pane of the Workspace: `HEAD` or its
    /// upstream moved, or the user refreshed (ADR 0079).
    pub(in crate::app) fn reload_git_graph(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
    ) {
        let tabs: Vec<PaneId> = self
            .git_graph
            .views
            .iter()
            .filter(|((connection, _), view)| {
                *connection == key && view.workspace_id == workspace_id
            })
            .map(|((_, pane_id), _)| *pane_id)
            .collect();
        for pane_id in tabs {
            self.request_git_graph_page(key, pane_id, false);
        }
    }

    /// Another repository's commits: what Git Graph loaded of the last one goes (ADR 0073).
    pub(in crate::app) fn forget_git_graph(
        &mut self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
    ) {
        let tabs: Vec<PaneId> = self
            .git_graph
            .views
            .iter_mut()
            .filter(|((connection, _), view)| {
                *connection == key && view.workspace_id == workspace_id
            })
            .map(|((_, pane_id), view)| {
                view.branch = None;
                view.selected_ref = None;
                view.selected_commit = None;
                view.selected_file = None;
                *pane_id
            })
            .collect();
        if let Some(connection) = self.connection_mut(key) {
            for pane_id in tabs {
                connection.resources.graph_logs.remove(&pane_id);
                connection.resources.pending_graph_logs.remove(&pane_id);
            }
            connection.resources.graph_generation += 1;
        }
    }

    fn select_git_graph_row(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        row_ix: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.git_graph.views.get_mut(&(key, pane_id)) else {
            return;
        };
        let id = view
            .table
            .read(cx)
            .delegate()
            .rows
            .get(row_ix)
            .map(|row| row.commit.id.clone());
        if id.is_some() && id != view.selected_commit {
            view.selected_commit = id;
            view.selected_file = None;
            view.all_branches = false;
            cx.notify();
        }
    }

    /// Selects the parent, or the nearest child above, of the commit in row `row_ix`, as
    /// IntelliJ's Right and Left do.
    pub(in crate::app) fn step_git_graph_commit(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        row_ix: usize,
        parent: bool,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.git_graph.views.get(&(key, pane_id)) else {
            return;
        };
        view.table.update(cx, |table, cx| {
            let rows = &table.delegate().rows;
            let Some(row) = rows.get(row_ix) else {
                return;
            };
            let target = if parent {
                row.commit
                    .parents
                    .first()
                    .and_then(|id| rows.iter().position(|row| row.commit.id == *id))
            } else {
                rows[..row_ix]
                    .iter()
                    .rposition(|child| child.commit.parents.contains(&row.commit.id))
            };
            if let Some(target) = target {
                table.set_selected_row(target, cx);
                table.scroll_to_row(target, cx);
            }
        });
    }

    fn step_selected_git_graph_commit(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        parent: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row_ix) = self
            .git_graph
            .views
            .get(&(key, pane_id))
            .and_then(|view| view.table.read(cx).selected_row())
        else {
            return;
        };
        self.step_git_graph_commit(key, pane_id, row_ix, parent, window, cx);
    }

    fn apply_git_graph_text(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.git_graph.views.get_mut(&(key, pane_id)) else {
            return;
        };
        let text = view.text.read(cx).value().trim().to_owned();
        if text == view.text_filter {
            return;
        }
        view.text_filter = text;
        self.request_git_graph_page(key, pane_id, false);
        cx.notify();
    }

    fn set_git_graph_branch(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        branch: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.git_graph.views.get_mut(&(key, pane_id)) else {
            return;
        };
        if view.branch == branch {
            return;
        }
        view.selected_ref.clone_from(&branch);
        view.branch = branch;
        self.request_git_graph_page(key, pane_id, false);
        cx.notify();
    }

    /// `Cmd+D`: the selected file of the selected commit, in the Diff Pane.
    fn show_git_graph_diff(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.git_graph.views.get(&(key, pane_id)) else {
            return;
        };
        let (Some(id), Some(path)) = (view.selected_commit.clone(), view.selected_file.clone())
        else {
            return;
        };
        let workspace_id = view.workspace_id;
        let commit = view
            .table
            .read(cx)
            .delegate()
            .rows
            .iter()
            .find(|row| row.commit.id == id)
            .map(|row| row.commit.clone());
        if let Some(commit) = commit {
            self.show_commit_diff(key, workspace_id, commit, path, window, cx);
        }
    }

    /// The Git Graph Pane: the branches, the commits under their filters, and the selected
    /// commit's files above its details, as IntelliJ lays out its log (ADR 0079).
    pub(in crate::app) fn render_git_graph(
        &self,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        pane_id: PaneId,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (Some(view), Some(connection)) = (
            self.git_graph.views.get(&(key, pane_id)),
            self.connection(key),
        ) else {
            return empty_state(t!("git_history.loading"), cx);
        };
        let git = connection.model.workspace_git.get(&workspace_id);
        let Some((git, repository)) =
            git.and_then(|git| Some((git, self.git_history_repository(key, workspace_id, git)?)))
        else {
            return empty_state(t!("git_graph.not_a_repository"), cx);
        };
        let resources = &connection.resources;
        if !resources.graph_logs.contains_key(&pane_id)
            && !resources.pending_graph_logs.contains_key(&pane_id)
        {
            // Asked here rather than once, so a reconnect that dropped the answer asks again.
            let owner = cx.weak_entity();
            cx.defer(move |cx| {
                let _ = owner.update(cx, |this, cx| {
                    this.request_git_graph_page(key, pane_id, false);
                    cx.notify();
                });
            });
        }
        let log = resources.graph_logs.get(&pane_id);
        let current = current_branch(repository);
        // The table takes the rows of a page that landed since it last did.
        let generation = resources.graph_generation;
        let pending = resources.pending_graph_logs.contains_key(&pane_id);
        let selected = view.selected_commit.clone();
        let (tint_head, filtered) = (view.branch.is_none(), !view.text_filter.is_empty());
        view.table.update(cx, |table, cx| {
            let delegate = table.delegate_mut();
            delegate.loading = pending;
            if delegate.generation == Some(generation) {
                return;
            }
            delegate.generation = Some(generation);
            delegate.tint_head = tint_head;
            delegate.filtered = filtered;
            match log {
                Some(Ok(log)) => {
                    delegate.rows = graph_rows(log, current);
                    delegate.more = log.more;
                    delegate.user_email.clone_from(&log.user_email);
                }
                _ => {
                    delegate.rows.clear();
                    delegate.more = false;
                }
            }
            let position = selected.as_ref().and_then(|id| {
                table
                    .delegate()
                    .rows
                    .iter()
                    .position(|row| row.commit.id == *id)
            });
            match position {
                Some(row_ix) if table.selected_row() != Some(row_ix) => {
                    table.set_selected_row(row_ix, cx);
                }
                Some(_) => {}
                None => table.clear_selection(cx),
            }
            cx.notify();
        });
        let refs: &[GitRef] = match log {
            Some(Ok(log)) => &log.refs,
            _ => &[],
        };
        let table = view.table.clone();
        // The subject takes what the other columns leave; a new width redoes the columns,
        // which a frame later draws (shortcut: it also undoes widths dragged by hand).
        let measure = canvas(
            move |bounds, _, cx| {
                let width = bounds.size.width;
                if (table.read(cx).delegate().width - width).abs() > px(1.) {
                    table.update(cx, |table, cx| {
                        table.delegate_mut().width = width;
                        table.refresh(cx);
                        cx.notify();
                    });
                }
            },
            |_, (), _, _| {},
        )
        .absolute()
        .size_full();
        let commits = match log {
            Some(Err(error)) => failed_state(error.clone(), cx),
            _ => div()
                .relative()
                .size_full()
                .child(
                    DataTable::new(&view.table)
                        .small()
                        .bordered(false)
                        .stripe(false),
                )
                .child(measure)
                .into_any_element(),
        };
        let commits = v_flex()
            .size_full()
            .child(self.render_git_graph_filters(view, key, workspace_id, pane_id, git, cx))
            .child(div().flex_1().min_h_0().child(commits));
        let branches = self.render_git_graph_branches(view, key, pane_id, refs, current, cx);
        let details = self.render_git_graph_commit(view, key, pane_id, refs, current, cx);
        div()
            .id(SharedString::from(format!(
                "git-graph-{}",
                pane_id.as_u64()
            )))
            .debug_selector(|| "git-graph".into())
            .key_context(CONTEXT)
            .flex_1()
            .min_h_0()
            .w_full()
            .bg(cx.theme().background)
            .on_action(cx.listener(move |this, _: &GoToParentCommit, window, cx| {
                this.step_selected_git_graph_commit(key, pane_id, true, window, cx);
            }))
            .on_action(cx.listener(move |this, _: &GoToChildCommit, window, cx| {
                this.step_selected_git_graph_commit(key, pane_id, false, window, cx);
            }))
            .on_action(
                cx.listener(move |this, _: &FocusGitGraphFilter, window, cx| {
                    if let Some(view) = this.git_graph.views.get(&(key, pane_id)) {
                        window.focus(&view.text.read(cx).focus_handle(cx), cx);
                    }
                }),
            )
            .on_action(cx.listener(move |this, _: &ShowGitGraphDiff, window, cx| {
                this.show_git_graph_diff(key, pane_id, window, cx);
            }))
            .child(
                h_resizable(SharedString::from(format!(
                    "git-graph-columns-{}",
                    pane_id.as_u64()
                )))
                .child(resizable_panel().size(px(180.)).child(branches))
                .child(resizable_panel().child(commits))
                .child(resizable_panel().size(px(300.)).child(details)),
            )
            .into_any_element()
    }

    /// Above the commits: the repository, when there are several, the text filter and the
    /// branch filter, as IntelliJ's log has them.
    fn render_git_graph_filters(
        &self,
        view: &GitGraphView,
        key: ConnectionKey,
        workspace_id: WorkspaceId,
        pane_id: PaneId,
        git: &WorkspaceGitSnapshot,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let (muted, border) = (theme.muted_foreground, theme.border);
        let owner = cx.weak_entity();
        let branch = match view.branch.as_deref() {
            Some(name) => h_flex()
                .debug_selector(|| "git-graph-branch-filter".into())
                .flex_none()
                .gap_1()
                .items_center()
                .text_xs()
                .child(t!("git_graph.branch_filter", branch = short_name(name)))
                .child(
                    Button::new("git-graph-clear-branch")
                        .debug_selector(|| "git-graph-clear-branch".into())
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::Close))
                        .tooltip(t!("git_graph.clear_branch"))
                        .accessibility_label(t!("git_graph.clear_branch"))
                        .on_click(move |_, _, cx| {
                            let _ = owner.update(cx, |this, cx| {
                                this.set_git_graph_branch(key, pane_id, None, cx);
                            });
                        }),
                )
                .into_any_element(),
            None => div()
                .flex_none()
                .text_xs()
                .text_color(muted)
                .child(t!("git_graph.branch_any"))
                .into_any_element(),
        };
        h_flex()
            .flex_none()
            .w_full()
            .h_9()
            .px_2()
            .gap_3()
            .items_center()
            .border_b_1()
            .border_color(border)
            .children(self.git_history_repository_menu(
                "git-graph-repository",
                key,
                workspace_id,
                git,
                cx,
            ))
            .child(
                div().w(rems(18.)).child(
                    Input::new(&view.text)
                        .xsmall()
                        .prefix(Icon::new(IconName::Search).size_3().text_color(muted)),
                ),
            )
            .child(branch)
            .into_any_element()
    }

    /// The branch tree, under its own filter field.
    fn render_git_graph_branches(
        &self,
        view: &GitGraphView,
        key: ConnectionKey,
        pane_id: PaneId,
        refs: &[GitRef],
        current: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let filter = view.search.read(cx).value().trim().to_owned();
        let mut list = v_flex().w_full().px_1().py_1();
        for row in branch_rows(refs, current, &filter, &view.folded) {
            list = list.child(self.render_branch_row(view, key, pane_id, row, cx));
        }
        v_flex()
            .size_full()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                div().flex_none().p_1().child(
                    Input::new(&view.search)
                        .xsmall()
                        .prefix(Icon::new(IconName::Search).size_3()),
                ),
            )
            .child(
                sidebar_list(
                    SharedString::from(format!("git-graph-branches-{}", pane_id.as_u64())),
                    list,
                )
                .flex_1()
                .min_h_0()
                .w_full(),
            )
            .into_any_element()
    }

    fn render_branch_row(
        &self,
        view: &GitGraphView,
        key: ConnectionKey,
        pane_id: PaneId,
        row: BranchRow,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let owner = cx.weak_entity();
        let muted = cx.theme().muted_foreground;
        match row {
            BranchRow::Folder {
                key: folder,
                label,
                depth,
                folded,
            } => {
                let label = match (depth, label.as_str()) {
                    (0, "local") => t!("git_graph.branches.local").into_owned(),
                    (0, "remote") => t!("git_graph.branches.remote").into_owned(),
                    (0, "tags") => t!("git_graph.branches.tags").into_owned(),
                    _ => label,
                };
                tree_row(
                    format!("git-graph-folder-{folder}").into(),
                    depth,
                    RowKind::Directory { open: !folded },
                    cx,
                )
                .child(Icon::new(IconName::Folder).size_4().text_color(muted))
                .child(div().min_w_0().flex_1().truncate().child(label))
                .on_click(move |_, _, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        if let Some(view) = this.git_graph.views.get_mut(&(key, pane_id))
                            && !view.folded.remove(&folder)
                        {
                            view.folded.insert(folder.clone());
                        }
                        cx.notify();
                    });
                })
                .into_any_element()
            }
            BranchRow::Ref {
                name,
                label,
                depth,
                kind,
                current,
            } => {
                let selected = view.selected_ref.as_deref() == Some(name.as_str());
                let (icon, color) = match kind {
                    RefKind::Head => (Glyph::GitCommitHorizontal, cx.theme().yellow),
                    RefKind::Tag => (Glyph::Tag, muted),
                    _ if current => (Glyph::GitBranch, cx.theme().yellow),
                    _ => (Glyph::GitBranch, muted),
                };
                let label = if kind == RefKind::Head {
                    t!("git_graph.branches.head").into_owned()
                } else {
                    label
                };
                tree_row(
                    format!("git-graph-ref-{name}").into(),
                    depth,
                    RowKind::File { selected },
                    cx,
                )
                .child(Icon::new(icon).size_4().text_color(color))
                .child(div().min_w_0().flex_1().truncate().child(label))
                .on_click(move |event, _, cx| {
                    let name = name.clone();
                    let _ = owner.update(cx, |this, cx| {
                        // A single click selects; a double click filters the log by it.
                        if event.click_count() >= 2 {
                            this.set_git_graph_branch(key, pane_id, Some(name), cx);
                        } else if let Some(view) = this.git_graph.views.get_mut(&(key, pane_id)) {
                            view.selected_ref = Some(name);
                            cx.notify();
                        }
                    });
                })
                .into_any_element()
            }
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Condr {
    /// The subjects a Git Graph Pane's table lists, in order.
    pub(in crate::app) fn git_graph_subjects(
        &self,
        key: ConnectionKey,
        pane_id: PaneId,
        cx: &App,
    ) -> Vec<String> {
        self.git_graph
            .views
            .get(&(key, pane_id))
            .map(|view| {
                view.table
                    .read(cx)
                    .delegate()
                    .rows
                    .iter()
                    .map(|row| row.commit.subject.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The ids a Git Graph Pane's table lists, in order.
    #[cfg(test)]
    pub(in crate::app) fn git_graph_ids(
        &self,
        key: ConnectionKey,
        pane_id: PaneId,
        cx: &App,
    ) -> Vec<String> {
        self.git_graph
            .views
            .get(&(key, pane_id))
            .map(|view| {
                let table = view.table.read(cx);
                table
                    .delegate()
                    .rows
                    .iter()
                    .map(|row| row.commit.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// A Git Graph Pane's table, to select a row as a click would.
    pub(in crate::app) fn git_graph_table(
        &self,
        key: ConnectionKey,
        pane_id: PaneId,
    ) -> Option<Entity<TableState<CommitTable>>> {
        Some(self.git_graph.views.get(&(key, pane_id))?.table.clone())
    }
}
