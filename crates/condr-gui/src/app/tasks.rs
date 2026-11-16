//! Tasks mode (ADR 0068): the main window's whole-window view of every Agent, in
//! columns by state, as a board or a list.

use super::sidebar::{
    CondrIconName, CondrSidebarIcon, SidebarStatusVisual, pane_sidebar_status, urgency,
};
use super::viewer_header::mode_toggle;
use super::*;
use rust_i18n::t;
use std::any::TypeId;

/// Which of Tasks mode's layouts the window shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum TasksLayout {
    #[default]
    Board,
    List,
}

/// How much a Board card says; the List is always compact.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum TasksDensity {
    #[default]
    Cards,
    Compact,
}

/// What Tasks mode shows: every Agent, one Device's, or one Workspace's together with
/// the Managed Worktrees made from it. A Device is kept by its Server's id, which
/// outlives connection order and restarts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum TasksScope {
    #[default]
    All,
    Device(ServerId),
    Workspace(ServerId, WorkspaceId),
}

/// The main window's Tasks mode as the GUI state file keeps it (ADR 0023).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct TasksPrefs {
    pub(super) on: bool,
    pub(super) layout: TasksLayout,
    pub(super) density: TasksDensity,
    pub(super) scope: TasksScope,
    /// One bit per column, by `TasksColumn::bit`.
    pub(super) folded: u8,
    /// Where each layout splits the cards from the interaction area, in thousandths of
    /// the Board's height or the List's width; `None` keeps the default.
    pub(super) split_board: Option<u16>,
    pub(super) split_list: Option<u16>,
}

/// The card whose Agent the interaction area shows: its Pane, the column it had when it
/// was selected, which it keeps until another is selected or the area closes, and
/// whether the keyboard is in its terminal or in the card area (ADR 0068).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TasksSelection {
    pub(super) key: ConnectionKey,
    pub(super) pane_id: PaneId,
    pub(super) column: Option<TasksColumn>,
    pub(super) keyboard: bool,
}

/// The Dock holding only the selected Agent's Pane: locked and never subscribed to, so
/// it sends no Layout command; zoom is Session state and is not borrowed (ADR 0068).
pub(super) struct TasksDock {
    pub(super) area: Entity<DockArea>,
    pub(super) pane: (ConnectionKey, PaneId),
}

/// The drag payload of the handle between the cards and the interaction area.
#[derive(Clone)]
pub(super) struct DraggedTasksSplit;

const DEFAULT_SPLIT_BOARD: u16 = 450;
const DEFAULT_SPLIT_LIST: u16 = 440;

impl TasksPrefs {
    pub(super) fn is_folded(self, column: TasksColumn) -> bool {
        self.folded & column.bit() != 0
    }
}

/// The columns, in their one order (ADR 0068). To do joins them with tasks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum TasksColumn {
    NeedsYou,
    Done,
    Working,
    Idle,
}

impl TasksColumn {
    pub(super) const ALL: [Self; 4] = [Self::NeedsYou, Self::Done, Self::Working, Self::Idle];

    /// The column an Agent's state puts it in: a bell, `Blocked` and `Error` need the
    /// person, `Unknown` sits with the idle.
    pub(super) fn of(status: SidebarStatusVisual) -> Self {
        match status.key {
            "bell" | "blocked" | "error" => Self::NeedsYou,
            "done" => Self::Done,
            "working" => Self::Working,
            _ => Self::Idle,
        }
    }

    pub(super) fn bit(self) -> u8 {
        1 << Self::ALL
            .iter()
            .position(|column| *column == self)
            .expect("every column is listed")
    }

    fn selector(self) -> &'static str {
        match self {
            Self::NeedsYou => "needs-you",
            Self::Done => "done",
            Self::Working => "working",
            Self::Idle => "idle",
        }
    }

    fn label(self) -> SharedString {
        match self {
            Self::NeedsYou => t!("tasks.column.needs_you"),
            Self::Done => t!("tasks.column.done"),
            Self::Working => t!("tasks.column.working"),
            Self::Idle => t!("tasks.column.idle"),
        }
        .into_owned()
        .into()
    }
}

/// One Agent's card: where it lives and how it shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TaskCard {
    pub(super) key: ConnectionKey,
    pub(super) workspace_id: WorkspaceId,
    pub(super) pane_id: PaneId,
    pub(super) status: SidebarStatusVisual,
}

/// Sorts a column by urgency; ties keep Session order, the cards coming in it.
pub(super) fn rank_column(cards: &mut [TaskCard]) {
    cards.sort_by_key(|card| urgency(card.status));
}

/// The layout and Tab actions, which act on a Tab Tasks mode does not show (ADR 0068).
pub(super) fn inert_in_tasks_mode(action: TypeId) -> bool {
    [
        TypeId::of::<NewTab>(),
        TypeId::of::<RenameTab>(),
        TypeId::of::<ClosePane>(),
        TypeId::of::<CloseTab>(),
        TypeId::of::<NextTab>(),
        TypeId::of::<PreviousTab>(),
        TypeId::of::<ActivateTab>(),
        TypeId::of::<SplitRight>(),
        TypeId::of::<SplitDown>(),
        TypeId::of::<FocusLeft>(),
        TypeId::of::<FocusRight>(),
        TypeId::of::<FocusUp>(),
        TypeId::of::<FocusDown>(),
        TypeId::of::<ResizeLeft>(),
        TypeId::of::<ResizeRight>(),
        TypeId::of::<ResizeUp>(),
        TypeId::of::<ResizeDown>(),
        TypeId::of::<SwapLeft>(),
        TypeId::of::<SwapRight>(),
        TypeId::of::<SwapUp>(),
        TypeId::of::<SwapDown>(),
        TypeId::of::<ToggleZoom>(),
        TypeId::of::<ToggleSidebar>(),
        TypeId::of::<ToggleChanges>(),
        TypeId::of::<NextWorkspace>(),
        TypeId::of::<PreviousWorkspace>(),
    ]
    .contains(&action)
}

impl Condr {
    /// Whether `view` shows Tasks mode, which only the main window does.
    pub(super) fn in_tasks_mode(&self, view: &WindowView) -> bool {
        view.scope == WindowScope::Main && view.tasks.on
    }

    /// Enters or leaves Tasks mode. Neither changes the View (ADR 0021): leaving shows
    /// the Workspace and Tab shown before, and entering marks nothing seen.
    pub(super) fn toggle_tasks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.window_view_mut(window);
        if view.scope != WindowScope::Main {
            return;
        }
        view.tasks.on = !view.tasks.on;
        if view.tasks.on {
            view.ports = None;
        }
        self.rebuild_window_dock(window, cx);
        self.sync_terminal_focus(window, cx);
        self.schedule_state_save(cx);
        cx.notify();
    }

    /// Makes each window's cached views before a frame renders them: GPUI tracks a
    /// cached view for invalidation from the frames after the one it was made in, so
    /// one made while a frame renders is never notified again.
    pub(super) fn prepare_cached_views(&mut self, cx: &mut Context<Self>) {
        let owner = cx.entity();
        for index in 0..self.windows.len() {
            if self.windows[index].sidebar.is_none() {
                let sidebar = cx.new(|cx| super::sidebar::LeftSidebar::new(&owner, cx));
                self.windows[index].sidebar = Some(sidebar);
            }
            if self.windows[index].scope == WindowScope::Main
                && self.windows[index].tasks_board.is_none()
            {
                let board = cx.new(|cx| TasksBoard::new(&owner, cx));
                self.windows[index].tasks_board = Some(board);
            }
        }
    }

    /// The Tasks row at the top of the main window's sidebar.
    pub(super) fn render_tasks_entry(&self, cx: &mut Context<Self>) -> AnyElement {
        let owner = cx.weak_entity();
        h_flex()
            .id("sidebar-tasks")
            .debug_selector(|| "sidebar-tasks".into())
            .h_8()
            .w_full()
            .px_2()
            .gap_2()
            .items_center()
            .rounded(cx.theme().radius)
            .text_sm()
            .font_semibold()
            .map(|this| super::change_tree::row_highlight(this, false, cx))
            .child(Icon::new(CondrIconName::SquareKanban).size_4())
            .child(t!("tasks.title").into_owned())
            .on_click(move |_, window, cx| {
                let _ = owner.update(cx, |this, cx| this.toggle_tasks(window, cx));
            })
            .into_any_element()
    }

    /// The Tasks icon heading the collapsed sidebar's avatars.
    pub(super) fn render_collapsed_tasks_entry(&self, cx: &mut Context<Self>) -> AnyElement {
        let owner = cx.weak_entity();
        Button::new("collapsed-tasks")
            .debug_selector(|| "collapsed-tasks".into())
            .ghost()
            .small()
            .icon(Icon::new(CondrIconName::SquareKanban))
            .tooltip_with_action(t!("tasks.title"), &ToggleTasks, Some(SHORTCUT_CONTEXT))
            .on_click(move |_, window, cx| {
                let _ = owner.update(cx, |this, cx| this.toggle_tasks(window, cx));
            })
            .into_any_element()
    }

    fn toggle_tasks_column(&mut self, column: TasksColumn, cx: &mut Context<Self>) {
        self.main_view_mut().tasks.folded ^= column.bit();
        self.schedule_state_save(cx);
        cx.notify();
    }

    pub(super) fn set_tasks_scope(&mut self, scope: TasksScope, cx: &mut Context<Self>) {
        self.main_view_mut().tasks.scope = scope;
        self.schedule_state_save(cx);
        cx.notify();
    }

    /// Unfolds a folded Needs you when an Agent that was not in it arrives; the other
    /// columns only count theirs (ADR 0068).
    pub(super) fn unfold_needs_you_for_arrivals(&mut self, cx: &mut Context<Self>) {
        let mut now = HashSet::new();
        for connection in &self.connections {
            for pane_id in connection.model.agents.keys() {
                if pane_sidebar_status(&connection.model, *pane_id)
                    .is_some_and(|status| TasksColumn::of(status) == TasksColumn::NeedsYou)
                {
                    now.insert((connection.key, *pane_id));
                }
            }
        }
        let arrived = now
            .iter()
            .any(|agent| !self.tasks_needs_you.contains(agent));
        self.tasks_needs_you = now;
        let tasks = &mut self.main_view_mut().tasks;
        if arrived && tasks.is_folded(TasksColumn::NeedsYou) {
            tasks.folded &= !TasksColumn::NeedsYou.bit();
            self.schedule_state_save(cx);
        }
    }

    /// The scope to show: a Workspace a connected Device no longer has, or a Device no
    /// longer configured, falls back to All. A Device that is only disconnected keeps it.
    pub(super) fn effective_tasks_scope(&self, scope: TasksScope) -> TasksScope {
        let connection = |server: ServerId| {
            self.connections
                .iter()
                .find(|connection| connection.model.server_id == Some(server))
        };
        match scope {
            TasksScope::All => scope,
            TasksScope::Device(server) if connection(server).is_some() => scope,
            TasksScope::Workspace(server, workspace_id) => match connection(server) {
                Some(connection) => match connection.presented_session() {
                    Some(session) if session.workspace(workspace_id).is_none() => TasksScope::All,
                    _ => scope,
                },
                None => TasksScope::All,
            },
            TasksScope::Device(_) => TasksScope::All,
        }
    }

    /// Whether `scope` shows `workspace` on `connection`: a repository's scope takes in
    /// the Managed Worktrees made from it, a worktree's only itself.
    fn tasks_scope_shows(
        scope: TasksScope,
        connection: &ServerConnection,
        workspace: &Workspace,
    ) -> bool {
        match scope {
            TasksScope::All => true,
            TasksScope::Device(server) => connection.model.server_id == Some(server),
            TasksScope::Workspace(server, workspace_id) => {
                connection.model.server_id == Some(server)
                    && (workspace.id() == workspace_id
                        || workspace.worktree().is_some_and(|association| {
                            association.parent_workspace_id() == workspace_id
                        }))
            }
        }
    }

    fn set_tasks_layout(&mut self, layout: TasksLayout, cx: &mut Context<Self>) {
        self.main_view_mut().tasks.layout = layout;
        self.schedule_state_save(cx);
        cx.notify();
    }

    fn set_tasks_density(&mut self, density: TasksDensity, cx: &mut Context<Self>) {
        self.main_view_mut().tasks.density = density;
        self.schedule_state_save(cx);
        cx.notify();
    }

    /// Every Agent on a connected Device in `scope`, by column, each column by urgency.
    /// The selected card keeps the column it had when it was selected.
    pub(super) fn task_cards(
        &self,
        scope: TasksScope,
        selection: Option<TasksSelection>,
    ) -> Vec<(TasksColumn, Vec<TaskCard>)> {
        let mut columns: Vec<(TasksColumn, Vec<TaskCard>)> = TasksColumn::ALL
            .iter()
            .map(|column| (*column, Vec::new()))
            .collect();
        for connection in &self.connections {
            let Some(session) = connection.presented_session() else {
                continue;
            };
            for workspace in session.workspaces() {
                if !Self::tasks_scope_shows(scope, connection, workspace) {
                    continue;
                }
                for pane in workspace.tabs().iter().flat_map(|tab| tab.panes()) {
                    let Some(status) = pane_sidebar_status(&connection.model, pane.id()) else {
                        continue;
                    };
                    let card = TaskCard {
                        key: connection.key,
                        workspace_id: workspace.id(),
                        pane_id: pane.id(),
                        status,
                    };
                    let column = selection
                        .filter(|selected| {
                            (selected.key, selected.pane_id) == (connection.key, pane.id())
                        })
                        .and_then(|selected| selected.column)
                        .unwrap_or_else(|| TasksColumn::of(status));
                    columns
                        .iter_mut()
                        .find(|(each, _)| *each == column)
                        .expect("every column is listed")
                        .1
                        .push(card);
                }
            }
        }
        for (_, cards) in &mut columns {
            rank_column(cards);
        }
        columns
    }

    /// Tasks mode's title row: back to the Workspaces, the mode's name, and how it shows.
    pub(super) fn tasks_title_bar(&self, view: &WindowView, cx: &mut Context<Self>) -> TitleBar {
        let theme = cx.theme();
        let prefs = view.tasks;
        let back_owner = cx.weak_entity();
        let back_label = t!("tasks.back");
        let back = Button::new("tasks-back")
            .debug_selector(|| "tasks-back".into())
            .ghost()
            .small()
            .icon(Icon::new(IconName::ChevronLeft))
            .label(back_label.clone())
            .tooltip_with_action(back_label, &ToggleTasks, Some(SHORTCUT_CONTEXT))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| {
                let _ = back_owner.update(cx, |this, cx| this.toggle_tasks(window, cx));
            });
        let layout_owner = cx.weak_entity();
        let layout = mode_toggle(
            "tasks-layout",
            Button::new("tasks-layout-list")
                .debug_selector(|| "tasks-layout-list".into())
                .label(t!("tasks.layout.list")),
            Button::new("tasks-layout-board")
                .debug_selector(|| "tasks-layout-board".into())
                .label(t!("tasks.layout.board")),
            prefs.layout == TasksLayout::Board,
            move |board, cx| {
                let layout = if board {
                    TasksLayout::Board
                } else {
                    TasksLayout::List
                };
                let _ = layout_owner.update(cx, |this, cx| this.set_tasks_layout(layout, cx));
            },
        );
        let density_owner = cx.weak_entity();
        let density = (prefs.layout == TasksLayout::Board).then(|| {
            mode_toggle(
                "tasks-density",
                Button::new("tasks-density-cards")
                    .debug_selector(|| "tasks-density-cards".into())
                    .label(t!("tasks.density.cards")),
                Button::new("tasks-density-compact")
                    .debug_selector(|| "tasks-density-compact".into())
                    .label(t!("tasks.density.compact")),
                prefs.density == TasksDensity::Compact,
                move |compact, cx| {
                    let density = if compact {
                        TasksDensity::Compact
                    } else {
                        TasksDensity::Cards
                    };
                    let _ =
                        density_owner.update(cx, |this, cx| this.set_tasks_density(density, cx));
                },
            )
        });
        TitleBar::new()
            .h(WORKSPACE_TITLE_BAR_HEIGHT)
            .bg(theme.sidebar)
            .border_color(theme.border)
            .child(
                h_flex()
                    .debug_selector(|| "tasks-title".into())
                    .h_full()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .pr_2()
                    .items_center()
                    .child(back)
                    .child(self.render_scope_menu(prefs.scope, cx))
                    .child(div().flex_1())
                    .children(density)
                    .child(layout),
            )
    }

    /// `Tasks › MacBook › api-server ▾`: the scope as a breadcrumb that opens a tree of
    /// All, the Devices and their Workspaces, worktrees under their repository, each
    /// with its Agent count.
    fn render_scope_menu(&self, scope: TasksScope, cx: &mut Context<Self>) -> AnyElement {
        let scope = self.effective_tasks_scope(scope);
        let count = |scope| -> usize {
            self.task_cards(scope, None)
                .iter()
                .map(|(_, cards)| cards.len())
                .sum()
        };
        let mut crumbs = vec![t!("tasks.title").into_owned()];
        let mut options = vec![(t!("tasks.scope.all").into_owned(), TasksScope::All)];
        for connection in &self.connections {
            let Some(server) = connection.model.server_id else {
                continue;
            };
            options.push((connection.label.clone(), TasksScope::Device(server)));
            if scope == TasksScope::Device(server) {
                crumbs.push(connection.label.clone());
            }
            let Some(session) = connection.presented_session() else {
                continue;
            };
            let parent_of = |workspace: &Workspace| {
                workspace
                    .worktree()
                    .map(|association| association.parent_workspace_id())
                    .filter(|parent| session.workspace(*parent).is_some())
            };
            for repository in session
                .workspaces()
                .iter()
                .filter(|each| parent_of(each).is_none())
            {
                let repository_scope = TasksScope::Workspace(server, repository.id());
                options.push((format!("\u{2003}{}", repository.name()), repository_scope));
                if scope == repository_scope {
                    crumbs.extend([connection.label.clone(), repository.name().to_owned()]);
                }
                for worktree in session
                    .workspaces()
                    .iter()
                    .filter(|each| parent_of(each) == Some(repository.id()))
                {
                    let worktree_scope = TasksScope::Workspace(server, worktree.id());
                    options.push((
                        format!("\u{2003}\u{2003}└ {}", worktree.name()),
                        worktree_scope,
                    ));
                    if scope == worktree_scope {
                        crumbs.extend([
                            connection.label.clone(),
                            repository.name().to_owned(),
                            worktree.name().to_owned(),
                        ]);
                    }
                }
            }
        }
        if scope == TasksScope::All {
            crumbs.push(t!("tasks.scope.all").into_owned());
        }
        let options: Vec<(String, TasksScope)> = options
            .into_iter()
            .map(|(name, each)| (format!("{name}  {}", count(each)), each))
            .collect();
        let owner = cx.weak_entity();
        Button::new("tasks-scope")
            .debug_selector(|| "tasks-scope".into())
            .ghost()
            .small()
            .label(crumbs.join(" › "))
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                options.iter().fold(menu, |menu, (name, each)| {
                    let (owner, each) = (owner.clone(), *each);
                    menu.item(
                        PopupMenuItem::new(name.clone())
                            .checked(each == scope)
                            .on_click(move |_, _, cx| {
                                let _ = owner.update(cx, |this, cx| this.set_tasks_scope(each, cx));
                            }),
                    )
                })
            })
            .into_any_element()
    }

    /// The card area, which `TasksBoard` caches (ADR 0068).
    pub(super) fn render_task_cards(
        &self,
        view: &WindowView,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let columns = self.task_cards(
            self.effective_tasks_scope(view.tasks.scope),
            view.tasks_selection,
        );
        let cards = match view.tasks.layout {
            TasksLayout::Board => self.render_tasks_board(&columns, view.tasks, cx),
            TasksLayout::List => self.render_tasks_list(&columns, view.tasks, cx),
        };
        let bands = self.render_connection_bands(view.tasks.scope, cx);
        v_flex()
            .debug_selector(|| "tasks-cards".into())
            .size_full()
            .children(bands)
            .track_focus(&view.tasks_focus)
            .on_key_down(cx.listener(Self::tasks_key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.focus_task_cards(window, cx)),
            )
            .child(cards)
            .into_any_element()
    }

    fn render_tasks_board(
        &self,
        columns: &[(TasksColumn, Vec<TaskCard>)],
        prefs: TasksPrefs,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let density = prefs.density;
        let theme = cx.theme().clone();
        h_flex()
            .id("tasks-board")
            .debug_selector(|| "tasks-board".into())
            .size_full()
            .items_start()
            .gap_3()
            .p_3()
            .overflow_x_scroll()
            .children(columns.iter().map(|(column, cards)| {
                let column = *column;
                if prefs.is_folded(column) {
                    return self.render_strip(column, cards.len(), cx);
                }
                v_flex()
                    .id(column.selector())
                    .debug_selector(move || format!("tasks-column-{}", column.selector()))
                    .flex_1()
                    .min_w(rems(14.))
                    .h_full()
                    .gap_2()
                    .child(self.column_heading(column, cards.len(), &theme, cx))
                    .child(
                        v_flex()
                            .id(SharedString::from(format!(
                                "tasks-column-cards-{}",
                                column.selector()
                            )))
                            .flex_1()
                            .min_h_0()
                            .gap_2()
                            .overflow_y_scroll()
                            .map(|this| {
                                if cards.is_empty() {
                                    this.child(nothing_here(&theme))
                                } else {
                                    this.children(cards.iter().map(|card| match density {
                                        TasksDensity::Cards => self.render_card(card, cx),
                                        TasksDensity::Compact => self.render_compact_card(card, cx),
                                    }))
                                }
                            }),
                    )
                    .into_any_element()
            }))
            .into_any_element()
    }

    /// A folded column: its count over its name, one letter a line, since GPUI does not
    /// rotate text; a click unfolds it.
    fn render_strip(
        &self,
        column: TasksColumn,
        count: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let owner = cx.weak_entity();
        let theme = cx.theme();
        v_flex()
            .id(SharedString::from(format!(
                "tasks-strip-{}",
                column.selector()
            )))
            .debug_selector(move || format!("tasks-strip-{}", column.selector()))
            .flex_none()
            .w(rems(2.25))
            .h_full()
            .items_center()
            .gap_1()
            .py_2()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .text_xs()
            .text_color(theme.muted_foreground)
            .cursor_pointer()
            .child(div().font_semibold().child(count.to_string()))
            .children(
                column
                    .label()
                    .chars()
                    .filter(|letter| !letter.is_whitespace())
                    .map(|letter| div().child(letter.to_string())),
            )
            .on_click(move |_, _, cx| {
                let _ = owner.update(cx, |this, cx| this.toggle_tasks_column(column, cx));
            })
            .into_any_element()
    }

    /// A column's or group's name and count; a click folds it.
    fn column_heading(
        &self,
        column: TasksColumn,
        count: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let owner = cx.weak_entity();
        h_flex()
            .id(SharedString::from(format!(
                "tasks-heading-{}",
                column.selector()
            )))
            .debug_selector(move || format!("tasks-heading-{}", column.selector()))
            .gap_1()
            .px_1()
            .text_xs()
            .font_semibold()
            .text_color(theme.muted_foreground)
            .cursor_pointer()
            .child(column.label())
            .child(div().font_normal().child(count.to_string()))
            .on_click(move |_, _, cx| {
                let _ = owner.update(cx, |this, cx| this.toggle_tasks_column(column, cx));
            })
    }

    fn render_tasks_list(
        &self,
        columns: &[(TasksColumn, Vec<TaskCard>)],
        prefs: TasksPrefs,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let header = list_row(
            "tasks-list-header",
            div(),
            t!("tasks.list.title").into_owned(),
            t!("tasks.list.workspace").into_owned(),
            t!("tasks.list.branch").into_owned(),
            t!("tasks.list.device").into_owned(),
            div().child(t!("tasks.list.changes").into_owned()),
            div().child(t!("tasks.list.state").into_owned()),
        )
        .text_xs()
        .text_color(theme.muted_foreground);
        let mut rows = vec![header.into_any_element()];
        for (column, cards) in columns {
            let folded = prefs.is_folded(*column);
            rows.push(
                self.column_heading(*column, cards.len(), &theme, cx)
                    .pt_2()
                    .child(
                        Icon::new(IconName::ChevronRight)
                            .size_3()
                            .when(!folded, |icon| icon.rotate(percentage(90. / 360.))),
                    )
                    .into_any_element(),
            );
            if !folded {
                rows.extend(cards.iter().map(|card| self.render_list_row(card, cx)));
            }
        }
        v_flex()
            .id("tasks-list")
            .debug_selector(|| "tasks-list".into())
            .size_full()
            .p_3()
            .gap_1()
            .overflow_y_scroll()
            .children(rows)
            .into_any_element()
    }

    /// With the sidebar hidden, ADR 0020's mark has nowhere to be: a band names each
    /// Device in the scope that is not connected, whose Agents the cards then leave out.
    fn render_connection_bands(
        &self,
        scope: TasksScope,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let scope = self.effective_tasks_scope(scope);
        let mut bands = Vec::new();
        for connection in &self.connections {
            let in_scope = match scope {
                TasksScope::All => true,
                TasksScope::Device(server) | TasksScope::Workspace(server, _) => {
                    connection.model.server_id == Some(server)
                }
            };
            if !in_scope || connection.status == ConnectionStatus::Connected {
                continue;
            }
            let key = connection.key;
            let reconnecting = connection.reconnect_deadline.is_some();
            let owner = cx.weak_entity();
            let status = super::sidebar::device_status(
                key,
                connection.status,
                connection.build_notice().is_some(),
                connection.error.is_some() && !reconnecting,
                cx,
            );
            let theme = cx.theme();
            bands.push(
                h_flex()
                    .debug_selector(move || format!("tasks-band-{key}"))
                    .flex_none()
                    .w_full()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .bg(theme.sidebar)
                    .text_sm()
                    .child(status)
                    .child(div().min_w_0().truncate().child(
                        t!("tasks.band.not_connected", device = connection.label).into_owned(),
                    ))
                    .when(reconnecting, |this| {
                        this.child(
                            div()
                                .text_color(theme.muted_foreground)
                                .child(t!("tasks.band.reconnecting").into_owned()),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        Button::new(SharedString::from(format!("tasks-band-reconnect-{key}")))
                            .debug_selector(move || format!("tasks-band-reconnect-{key}"))
                            .ghost()
                            .xsmall()
                            .label(t!("tasks.band.reconnect"))
                            .on_click(move |_, window, cx| {
                                let _ = owner
                                    .update(cx, |this, cx| this.connect_server(key, window, cx));
                            }),
                    )
                    .into_any_element(),
            );
        }
        bands
    }

    /// What a card shows, gathered once for every density.
    fn card_facts(&self, card: &TaskCard) -> Option<CardFacts> {
        let connection = self.connection(card.key)?;
        let workspace = connection
            .presented_session()?
            .workspace(card.workspace_id)?;
        let agent = connection.model.agents.get(&card.pane_id)?;
        let kind = agent.kind;
        let title = connection
            .model
            .terminal_titles
            .get(&card.pane_id)
            .cloned()
            .unwrap_or_else(|| kind.label().to_owned());
        let git = connection.model.workspace_git.get(&card.workspace_id);
        let branch = git.and_then(|git| git.branch.clone());
        let (added, deleted) = git.map_or((0, 0), |git| {
            git.changes
                .entries
                .iter()
                .filter_map(|entry| entry.stat)
                .fold((0, 0), |(added, deleted), stat| {
                    (added + stat.added, deleted + stat.deleted)
                })
        });
        let ahead = git
            .and_then(|git| git.upstream)
            .map_or(0, |upstream| upstream.ahead);
        Some(CardFacts {
            kind,
            title,
            workspace: workspace.name().to_owned(),
            branch,
            device: connection.label.clone(),
            added,
            deleted,
            ahead,
            detail: agent.detail.clone(),
        })
    }

    fn card_icon(&self, card: &TaskCard, kind: AgentKind, cx: &mut Context<Self>) -> AnyElement {
        let (key, pane) = (card.key, card.pane_id.as_u64());
        CondrSidebarIcon::agent(
            kind,
            card.status,
            format!("tasks-status-{key}-{pane}-{}", card.status.key),
            format!("{}: {}", kind.label(), card.status.label),
        )
        .render(cx)
    }

    fn render_card(&self, card: &TaskCard, cx: &mut Context<Self>) -> AnyElement {
        let Some(facts) = self.card_facts(card) else {
            return div().into_any_element();
        };
        let icon = self.card_icon(card, facts.kind, cx);
        let (card_key, card_pane) = (card.key, card.pane_id);
        let selected = self
            .main_view()
            .tasks_selection
            .is_some_and(|selected| (selected.key, selected.pane_id) == (card_key, card_pane));
        let on_click = cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.click_task_card(card_key, card_pane, window, cx)
        });
        let theme = cx.theme();
        let (key, pane) = (card.key, card.pane_id.as_u64());
        let place = match &facts.branch {
            Some(branch) => format!("{} · {branch}", facts.workspace),
            None => facts.workspace.clone(),
        };
        v_flex()
            .id(SharedString::from(format!("tasks-card-{key}-{pane}")))
            .debug_selector(move || format!("tasks-card-{key}-{pane}"))
            .w_full()
            .gap_1()
            .p_2()
            .rounded(theme.radius)
            .border_1()
            .border_color(if selected {
                theme.primary
            } else {
                theme.border
            })
            .bg(theme.sidebar)
            .cursor_pointer()
            .on_click(on_click)
            .child(
                h_flex()
                    .gap_2()
                    .items_start()
                    .child(icon)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_semibold()
                            .line_clamp(2)
                            .child(facts.title),
                    )
                    .children(state_name(card.status, cx)),
            )
            .child(
                h_flex()
                    .gap_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(div().min_w_0().truncate().child(place))
                    .child(device_tag(facts.device, theme)),
            )
            .children(change_counts(
                facts.added,
                facts.deleted,
                facts.ahead,
                theme,
            ))
            .children(facts.detail.map(|detail| {
                div()
                    .text_xs()
                    .text_color(card.status.tone.color(cx))
                    .child(detail)
            }))
            .into_any_element()
    }

    fn render_compact_card(&self, card: &TaskCard, cx: &mut Context<Self>) -> AnyElement {
        let Some(facts) = self.card_facts(card) else {
            return div().into_any_element();
        };
        let icon = self.card_icon(card, facts.kind, cx);
        let (card_key, card_pane) = (card.key, card.pane_id);
        let selected = self
            .main_view()
            .tasks_selection
            .is_some_and(|selected| (selected.key, selected.pane_id) == (card_key, card_pane));
        let on_click = cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.click_task_card(card_key, card_pane, window, cx)
        });
        let theme = cx.theme();
        let (key, pane) = (card.key, card.pane_id.as_u64());
        v_flex()
            .id(SharedString::from(format!("tasks-card-{key}-{pane}")))
            .debug_selector(move || format!("tasks-card-{key}-{pane}"))
            .w_full()
            .px_1()
            .py_0p5()
            .rounded(theme.radius)
            .border_1()
            .border_color(if selected {
                theme.primary
            } else {
                gpui_kit::transparent_black()
            })
            .cursor_pointer()
            .on_click(on_click)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .text_sm()
                    .child(icon)
                    .child(div().flex_1().min_w_0().truncate().child(facts.title))
                    .children(state_name(card.status, cx)),
            )
            .children(facts.detail.map(|detail| {
                div()
                    .pl(rems(1.5))
                    .text_xs()
                    .truncate()
                    .text_color(card.status.tone.color(cx))
                    .child(detail)
            }))
            .into_any_element()
    }

    fn render_list_row(&self, card: &TaskCard, cx: &mut Context<Self>) -> AnyElement {
        let Some(facts) = self.card_facts(card) else {
            return div().into_any_element();
        };
        let icon = self.card_icon(card, facts.kind, cx);
        let (card_key, card_pane) = (card.key, card.pane_id);
        let selected = self
            .main_view()
            .tasks_selection
            .is_some_and(|selected| (selected.key, selected.pane_id) == (card_key, card_pane));
        let on_click = cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.click_task_card(card_key, card_pane, window, cx)
        });
        let theme = cx.theme();
        let (key, pane) = (card.key, card.pane_id.as_u64());
        let state = match &facts.detail {
            Some(detail) => div()
                .text_color(card.status.tone.color(cx))
                .child(detail.clone()),
            None => div().children(state_name(card.status, cx)),
        };
        list_row(
            format!("tasks-row-{key}-{pane}"),
            icon,
            facts.title,
            facts.workspace,
            facts.branch.unwrap_or_default(),
            facts.device,
            h_flex().children(change_counts(
                facts.added,
                facts.deleted,
                facts.ahead,
                theme,
            )),
            state,
        )
        .text_sm()
        .rounded(theme.radius)
        .border_1()
        .border_color(if selected {
            theme.primary
        } else {
            gpui_kit::transparent_black()
        })
        .cursor_pointer()
        .on_click(on_click)
        .into_any_element()
    }
}

/// What a card shows of its Agent.
struct CardFacts {
    kind: AgentKind,
    title: String,
    workspace: String,
    branch: Option<String>,
    device: String,
    added: u32,
    deleted: u32,
    ahead: u32,
    detail: Option<String>,
}

fn nothing_here(theme: &Theme) -> Div {
    div()
        .px_1()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(t!("tasks.nothing_here").into_owned())
}

/// The state's name in its colour, absent for a quiet Agent (ADR 0067).
fn state_name(status: SidebarStatusVisual, cx: &App) -> Option<Div> {
    (!matches!(status.key, "idle" | "unknown")).then(|| {
        div()
            .flex_none()
            .text_xs()
            .text_color(status.tone.color(cx))
            .child(status.label)
    })
}

fn device_tag(device: String, theme: &Theme) -> Div {
    div()
        .flex_none()
        .px_1()
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .child(device)
}

/// `+12 −3 ↑1`, or nothing for a clean Workspace in step with its upstream.
fn change_counts(added: u32, deleted: u32, ahead: u32, theme: &Theme) -> Option<Div> {
    (added > 0 || deleted > 0 || ahead > 0).then(|| {
        h_flex()
            .gap_1()
            .text_xs()
            .when(added > 0 || deleted > 0, |this| {
                this.child(div().text_color(theme.success).child(format!("+{added}")))
                    .child(div().text_color(theme.danger).child(format!("−{deleted}")))
            })
            .when(ahead > 0, |this| {
                this.child(div().text_color(theme.success).child(format!("↑{ahead}")))
            })
    })
}

#[allow(clippy::too_many_arguments)]
fn list_row(
    id: impl Into<SharedString>,
    icon: impl IntoElement,
    title: String,
    workspace: String,
    branch: String,
    device: String,
    changes: impl IntoElement,
    state: impl IntoElement,
) -> Stateful<Div> {
    let id = id.into();
    let selector = id.clone();
    h_flex()
        .id(id)
        .debug_selector(move || selector.to_string())
        .w_full()
        .gap_2()
        .px_1()
        .py_0p5()
        .items_center()
        .child(div().flex_none().w(rems(1.25)).child(icon))
        .child(div().flex_1().min_w_0().truncate().child(title))
        .child(div().flex_none().w(rems(9.)).truncate().child(workspace))
        .child(div().flex_none().w(rems(9.)).truncate().child(branch))
        .child(div().flex_none().w(rems(6.)).truncate().child(device))
        .child(div().flex_none().w(rems(6.)).child(changes))
        .child(div().flex_none().w(rems(12.)).truncate().child(state))
}

/// The card area as a view of its own, cached as the sidebar is: a terminal frame
/// re-renders `Condr` and reuses the cards, which render again when `Condr` notifies.
pub(super) struct TasksBoard {
    owner: WeakEntity<Condr>,
    _owner_changed: Subscription,
}

impl TasksBoard {
    pub(super) fn new(owner: &Entity<Condr>, cx: &mut Context<Self>) -> Self {
        Self {
            owner: owner.downgrade(),
            _owner_changed: cx.observe(owner, |_, _, cx| cx.notify()),
        }
    }
}

impl Render for TasksBoard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.owner
            .update(cx, |condr, cx| {
                if !condr.owns_window(window) {
                    return div().into_any_element();
                }
                #[cfg(feature = "test-support")]
                condr
                    .tasks_board_render_count
                    .set(condr.tasks_board_render_count.get() + 1);
                let condr = &*condr;
                condr.render_task_cards(condr.window_view(window), cx)
            })
            .unwrap_or_else(|_| div().into_any_element())
    }
}

/// The interaction area: selecting, the single-Pane Dock, the keyboard and the split.
impl Condr {
    /// Selects a card and opens the interaction area on its Agent's Pane. With
    /// `keyboard`, its terminal takes the keyboard, and with it the PTY's sizing (ADR
    /// 0036); without, the area previews the Pane and the card area keeps the keyboard.
    pub(super) fn select_task_card(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.connection(key) else {
            return;
        };
        let column = pane_sidebar_status(&connection.model, pane_id).map(TasksColumn::of);
        let shown = connection
            .presented_session()
            .and_then(|session| session.workspace_for_pane(pane_id))
            .is_some_and(|workspace| {
                Self::tasks_scope_shows(
                    self.effective_tasks_scope(self.main_view().tasks.scope),
                    connection,
                    workspace,
                )
            });
        let view = self.main_view_mut();
        // A notification's Agent outside the scope widens it to All first.
        if !shown {
            view.tasks.scope = TasksScope::All;
        }
        let kept = view
            .tasks_selection
            .filter(|selected| (selected.key, selected.pane_id) == (key, pane_id))
            .and_then(|selected| selected.column);
        view.tasks_selection = Some(TasksSelection {
            key,
            pane_id,
            column: kept.or(column),
            keyboard,
        });
        self.rebuild_window_dock(window, cx);
        if keyboard {
            self.focus_tasks_terminal(key, pane_id, window, cx);
        } else {
            self.main_view().tasks_focus.clone().focus(window, cx);
        }
        self.sync_terminal_focus(window, cx);
        self.schedule_state_save(cx);
        cx.notify();
    }

    fn focus_tasks_terminal(
        &self,
        key: ConnectionKey,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(focus) = self
            .panels
            .get(&(key, pane_id))
            .map(|panel| panel.read(cx).focus_handle.clone())
        else {
            return;
        };
        let owner = cx.weak_entity();
        // The Panel mounts with the next render; focus it then and report the focus.
        window.defer(cx, move |window, cx| {
            focus.focus(window, cx);
            let _ = owner.update(cx, |this, cx| this.sync_terminal_focus(window, cx));
        });
    }

    /// Closes the interaction area, which ends the selection and leaves no terminal
    /// with the keyboard.
    pub(super) fn close_task_area(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.main_view_mut();
        view.tasks_selection = None;
        view.tasks_dock = None;
        self.terminal_input.target = None;
        self.main_view().tasks_focus.clone().focus(window, cx);
        self.sync_terminal_focus(window, cx);
        cx.notify();
    }

    /// A click on a card: the selected one closes the area, another opens it with the
    /// keyboard in its terminal.
    fn click_task_card(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = self
            .main_view()
            .tasks_selection
            .is_some_and(|selected| (selected.key, selected.pane_id) == (key, pane_id));
        if selected {
            self.close_task_area(window, cx);
        } else {
            self.select_task_card(key, pane_id, true, window, cx);
        }
    }

    /// The Agent cards in the order `next-agent` and the arrows step through: column
    /// by column, each by urgency.
    fn task_card_order(&self) -> Vec<(ConnectionKey, PaneId)> {
        let view = self.main_view();
        self.task_cards(
            self.effective_tasks_scope(view.tasks.scope),
            view.tasks_selection,
        )
        .into_iter()
        .flat_map(|(_, cards)| cards)
        .map(|card| (card.key, card.pane_id))
        .collect()
    }

    /// Steps the selection through the cards, wrapping at both ends; from no selection,
    /// forward starts at the first card and back at the last.
    pub(super) fn step_task_card(
        &mut self,
        step: isize,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let order = self.task_card_order();
        if order.is_empty() {
            return;
        }
        let current = self.main_view().tasks_selection.and_then(|selected| {
            order
                .iter()
                .position(|card| *card == (selected.key, selected.pane_id))
        });
        let next = match current {
            Some(ix) => (ix as isize + step).rem_euclid(order.len() as isize) as usize,
            None if step > 0 => 0,
            None => order.len() - 1,
        };
        let (key, pane_id) = order[next];
        self.select_task_card(key, pane_id, keyboard, window, cx);
    }

    /// The card area's keys: the arrows move the selection and keep the keyboard here,
    /// so neither an arrow nor Enter reaches an Agent by accident; Enter gives the
    /// selected terminal the keyboard.
    fn tasks_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let stroke = &event.keystroke;
        if stroke.modifiers.modified() {
            return;
        }
        match stroke.key.as_str() {
            "up" | "left" => self.step_task_card(-1, false, window, cx),
            "down" | "right" => self.step_task_card(1, false, window, cx),
            "enter" => {
                if let Some(selected) = self.main_view().tasks_selection {
                    self.select_task_card(selected.key, selected.pane_id, true, window, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    /// A press on the card area's background takes the keyboard back from the terminal.
    fn focus_task_cards(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.main_view().tasks_focus.clone();
        if let Some(selected) = self.main_view_mut().tasks_selection.as_mut() {
            selected.keyboard = false;
        }
        self.terminal_input.target = None;
        focus.focus(window, cx);
        self.sync_terminal_focus(window, cx);
        cx.notify();
    }

    /// Leaves Tasks mode, for Go to's places and Open in Workspace.
    pub(super) fn leave_tasks_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.main_view().tasks.on {
            self.main_view_mut().tasks.on = false;
            self.main_view_mut().tasks_dock = None;
            if self.is_main(window) {
                self.rebuild_window_dock(window, cx);
            }
            self.schedule_state_save(cx);
            cx.notify();
        }
    }

    fn open_selected_in_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selected) = self.main_view().tasks_selection else {
            return;
        };
        self.main_view_mut().tasks_selection = None;
        self.leave_tasks_mode(window, cx);
        self.select_pane(selected.key, selected.pane_id, window, cx);
    }

    /// Whether the interaction area only previews `pane_id`: then the Client sends it no
    /// size, so stepping through cards resizes no Agent (ADR 0068).
    pub(super) fn tasks_previews(&self, key: ConnectionKey, pane_id: PaneId) -> bool {
        let view = self.main_view();
        view.tasks.on
            && view.tasks_selection.is_some_and(|selected| {
                (selected.key, selected.pane_id) == (key, pane_id) && !selected.keyboard
            })
    }

    /// The other window that draws the selected Pane's Tab, if one does.
    fn tasks_pane_shown_elsewhere(
        &self,
        selected: TasksSelection,
    ) -> Option<(WindowScope, AnyWindowHandle)> {
        let tab_id = self
            .connection(selected.key)?
            .session()?
            .workspace_for_pane(selected.pane_id)?
            .tabs()
            .iter()
            .find(|tab| tab.panes().iter().any(|pane| pane.id() == selected.pane_id))?
            .id();
        self.windows.iter().skip(1).find_map(|view| {
            let (key, _, _, shown) = self.presented_in(view)?;
            ((key, shown) == (selected.key, tab_id)).then_some((view.scope, view.handle))
        })
    }

    /// The Dock part of `rebuild_window_dock` in Tasks mode: no Tab's surface, and a
    /// Dock of the selected Pane alone, unless another window draws it.
    pub(super) fn rebuild_tasks_dock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let input_window = self.last_active == window.window_handle();
        self.window_view_mut(window).dock_surface = None;
        // A Pane that closed takes the area with it.
        if let Some(selected) = self.main_view().tasks_selection
            && self
                .connection(selected.key)
                .and_then(ServerConnection::session)
                .and_then(|session| session.workspace_for_pane(selected.pane_id))
                .is_none()
        {
            let view = self.main_view_mut();
            view.tasks_selection = None;
            view.tasks_dock = None;
        }
        let selection = self.main_view().tasks_selection;
        if input_window {
            self.terminal_input.target = selection
                .filter(|selected| selected.keyboard)
                .map(|selected| (selected.key, selected.pane_id));
        }
        let Some(selected) = selection else {
            self.main_view_mut().tasks_dock = None;
            return;
        };
        if self.tasks_pane_shown_elsewhere(selected).is_some() {
            self.main_view_mut().tasks_dock = None;
            return;
        }
        let pane = (selected.key, selected.pane_id);
        if self
            .main_view()
            .tasks_dock
            .as_ref()
            .is_some_and(|dock| dock.pane == pane)
        {
            return;
        }
        let size = {
            let workspace = self.main_view().workspace_size;
            size(
                workspace.width.max(px(1.)),
                (workspace.height / 2.).max(px(1.)),
            )
        };
        let layout =
            self.build_dock_layout(selected.key, &PaneLayout::Pane(selected.pane_id), size, cx);
        let area = cx.new(|cx| {
            DockArea::new("condr-tasks-dock", None, window, cx)
                .with_renderer(Rc::new(CondrDockRenderer))
        });
        area.update(cx, |dock, cx| {
            dock.set_locked(true, window, cx);
            dock.set_center(layout, window, cx);
        });
        self.main_view_mut().tasks_dock = Some(TasksDock { area, pane });
    }

    /// The cards, and below the Board or beside the List the interaction area, with a
    /// handle between them.
    pub(super) fn render_tasks_body(
        &self,
        view: &WindowView,
        board: Entity<TasksBoard>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cards = AnyView::from(board).cached(StyleRefinement::default().size_full());
        let Some(selected) = view.tasks_selection else {
            return div().size_full().child(cards).into_any_element();
        };
        let area = self.render_tasks_area(view, selected, cx);
        let board_layout = view.tasks.layout == TasksLayout::Board;
        let split = if board_layout {
            view.tasks.split_board.unwrap_or(DEFAULT_SPLIT_BOARD)
        } else {
            view.tasks.split_list.unwrap_or(DEFAULT_SPLIT_LIST)
        };
        let fraction = relative(f32::from(split) / 1000.);
        let handle = div()
            .id("tasks-split")
            .debug_selector(|| "tasks-split".into())
            .flex_none()
            .bg(cx.theme().border)
            .map(|this| {
                if board_layout {
                    this.h(px(1.)).w_full().cursor_row_resize()
                } else {
                    this.w(px(1.)).h_full().cursor_col_resize()
                }
            })
            .on_drag(DraggedTasksSplit, |_, _, _, cx| cx.new(|_| EmptyView));
        let container = if board_layout { v_flex() } else { h_flex() };
        container
            .size_full()
            .on_drag_move(cx.listener(
                move |this, event: &DragMoveEvent<DraggedTasksSplit>, _, cx| {
                    let bounds = event.bounds;
                    let position = event.event.position;
                    let fraction = if board_layout {
                        (position.y - bounds.top()) / bounds.size.height
                    } else {
                        (position.x - bounds.left()) / bounds.size.width
                    };
                    let split = Some((fraction.clamp(0.2, 0.8) * 1000.).round() as u16);
                    let prefs = &mut this.main_view_mut().tasks;
                    if board_layout {
                        prefs.split_board = split;
                    } else {
                        prefs.split_list = split;
                    }
                    this.schedule_state_save(cx);
                    cx.notify();
                },
            ))
            .child(
                div()
                    .flex_none()
                    .map(|this| {
                        if board_layout {
                            this.h(fraction).w_full()
                        } else {
                            this.w(fraction).h_full()
                        }
                    })
                    .child(cards),
            )
            .child(handle)
            .child(div().flex_1().min_w_0().min_h_0().child(area))
            .into_any_element()
    }

    /// The interaction area: a header of its own over the selected Agent's Pane.
    fn render_tasks_area(
        &self,
        view: &WindowView,
        selected: TasksSelection,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (key, pane) = (selected.key, selected.pane_id);
        let connection = self.connection(key);
        let agent = connection.and_then(|connection| connection.model.agents.get(&pane));
        let card = connection.and_then(|connection| {
            let workspace = connection.presented_session()?.workspace_for_pane(pane)?;
            Some(TaskCard {
                key,
                workspace_id: workspace.id(),
                pane_id: pane,
                status: pane_sidebar_status(&connection.model, pane)?,
            })
        });
        let facts = card.as_ref().and_then(|card| self.card_facts(card));
        let icon = card
            .as_ref()
            .zip(facts.as_ref())
            .map(|(card, facts)| self.card_icon(card, facts.kind, cx));
        let theme = cx.theme();
        let title = match (&facts, agent) {
            (Some(facts), Some(_)) => facts.title.clone(),
            _ => connection
                .and_then(|connection| connection.model.terminal_titles.get(&pane).cloned())
                .unwrap_or_default(),
        };
        let place = facts.as_ref().map(|facts| {
            let mut place = vec![facts.workspace.clone()];
            place.extend(facts.branch.clone());
            place.push(facts.kind.label().to_owned());
            place.push(facts.device.clone());
            place.join(" · ")
        });
        let open_owner = cx.weak_entity();
        let close_owner = cx.weak_entity();
        let header = h_flex()
            .debug_selector(|| "tasks-area-header".into())
            .flex_none()
            .h(rems(2.25))
            .w_full()
            .gap_2()
            .px_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .children(icon)
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .font_semibold()
                    .child(title),
            )
            .children(place.map(|place| {
                div()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(place)
            }))
            .when(agent.is_none(), |this| {
                this.child(
                    div()
                        .debug_selector(|| "tasks-agent-exited".into())
                        .flex_none()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(t!("tasks.area.agent_exited").into_owned()),
                )
            })
            .child(div().flex_1())
            .child(
                Button::new("tasks-open-in-workspace")
                    .debug_selector(|| "tasks-open-in-workspace".into())
                    .ghost()
                    .xsmall()
                    .label(t!("tasks.area.open_in_workspace"))
                    .on_click(move |_, window, cx| {
                        let _ = open_owner
                            .update(cx, |this, cx| this.open_selected_in_workspace(window, cx));
                    }),
            )
            .child(
                Button::new("tasks-area-close")
                    .debug_selector(|| "tasks-area-close".into())
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::Close))
                    .tooltip(t!("tasks.area.close"))
                    .on_click(move |_, window, cx| {
                        let _ = close_owner.update(cx, |this, cx| this.close_task_area(window, cx));
                    }),
            );
        let body = match self.tasks_pane_shown_elsewhere(selected) {
            Some((scope, handle)) => self.render_shown_elsewhere(scope, handle, cx),
            None => match &view.tasks_dock {
                Some(dock) => div()
                    .debug_selector(|| "tasks-area-dock".into())
                    .size_full()
                    .child(dock.area.clone())
                    .into_any_element(),
                None => div().into_any_element(),
            },
        };
        // A Device retrying on its own veils its Pane, as its Workspace is in normal mode.
        let veil = connection.and_then(|connection| self.render_reconnect_veil(connection, cx));
        v_flex()
            .id("tasks-area")
            .debug_selector(|| "tasks-area".into())
            .size_full()
            .child(header)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(body)
                    .children(veil),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: it would glob in `gpui_kit::test` and shadow the built-in attribute.
    use super::super::sidebar::{SidebarGlyph, SidebarIconTone};
    use super::super::{
        ClosePane, NextAgent, OpenSettings, SplitRight, ToggleSidebar, ToggleTasks,
    };
    use super::{SidebarStatusVisual, TaskCard, TasksColumn, inert_in_tasks_mode, rank_column};
    use condr_core::{PaneId, WorkspaceId};
    use std::any::TypeId;

    fn status(key: &'static str) -> SidebarStatusVisual {
        SidebarStatusVisual {
            glyph: SidebarGlyph::Circle,
            tone: SidebarIconTone::Muted,
            key,
            label: key,
        }
    }

    #[test]
    fn states_fall_into_their_columns() {
        for (key, column) in [
            ("bell", TasksColumn::NeedsYou),
            ("blocked", TasksColumn::NeedsYou),
            ("error", TasksColumn::NeedsYou),
            ("done", TasksColumn::Done),
            ("working", TasksColumn::Working),
            ("idle", TasksColumn::Idle),
            ("unknown", TasksColumn::Idle),
        ] {
            assert_eq!(TasksColumn::of(status(key)), column, "{key}");
        }
    }

    #[test]
    fn a_column_sorts_by_urgency_and_keeps_session_order() {
        let card = |pane: u64, key| TaskCard {
            key: 1,
            workspace_id: WorkspaceId::from_u64(1),
            pane_id: PaneId::from_u64(pane),
            status: status(key),
        };
        let mut cards = [
            card(1, "error"),
            card(2, "blocked"),
            card(3, "bell"),
            card(4, "blocked"),
        ];
        rank_column(&mut cards);
        let panes: Vec<u64> = cards.iter().map(|card| card.pane_id.as_u64()).collect();
        assert_eq!(panes, [3, 2, 4, 1]);
    }

    #[test]
    fn layout_and_tab_actions_are_inert() {
        assert!(inert_in_tasks_mode(TypeId::of::<SplitRight>()));
        assert!(inert_in_tasks_mode(TypeId::of::<ClosePane>()));
        assert!(inert_in_tasks_mode(TypeId::of::<ToggleSidebar>()));
        assert!(!inert_in_tasks_mode(TypeId::of::<NextAgent>()));
        assert!(!inert_in_tasks_mode(TypeId::of::<OpenSettings>()));
        assert!(!inert_in_tasks_mode(TypeId::of::<ToggleTasks>()));
    }
}
