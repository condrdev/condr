//! Tasks mode (ADR 0068): the page of every Agent and every Task, in columns by state,
//! as a Board or a Table, in the main window or the Tasks window (ADR 0069). This root holds what its parts share and
//! entering and leaving it; each part is a submodule.

use super::sidebar::{
    CondrIconName, CondrSidebarIcon, SidebarIconTone, SidebarStatusVisual, pane_sidebar_status,
    urgency,
};
use super::*;
use condr_core::TaskId;
use rust_i18n::t;
use std::any::TypeId;
use std::path::Path;

mod archive;
mod area;
mod board;
mod card;
mod columns;
mod form;
mod form_area;
mod requests;
mod sidebar_entry;
mod start;
mod table;
mod todo;
mod toolbar;

pub(in crate::app) use board::TasksBoard;
pub(in crate::app) use form::{FormField, TaskForm};
pub(in crate::app) use requests::{AGENT_WAIT, AgentRequest, DeviceTasks, TaskRequest};
pub(in crate::app) use start::Where;
// What the submodules share with one another.
use card::device_tag;
use form::saved_task;
use start::{ContinueBlock, WorktreeBlock, default_branch, folder_name, starts_tasks};
use table::{ListFit, list_row};

/// `gpui_kit::Task` is the executor's.
type TaskRecord = condr_core::Task;

/// The drag payload of a To do card, which drops onto another of its Device's.
#[derive(Clone)]
pub(super) struct DraggedTodo {
    key: ConnectionKey,
    task: TaskId,
}

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

/// How the main window's Tasks mode shows, as the GUI state file keeps it (ADR 0023);
/// whether it shows is the window's `ContentPage`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct TasksPrefs {
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
    /// A started Task's own card, shown while its Workspace has no Agent, on that
    /// Workspace's focused Pane; `None` for an Agent's card.
    pub(super) task: Option<TaskId>,
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

pub(super) const DEFAULT_SPLIT_BOARD: u16 = 450;

const DEFAULT_SPLIT_LIST: u16 = 440;

impl TasksPrefs {
    pub(super) fn is_folded(self, column: TasksColumn) -> bool {
        self.folded & column.bit() != 0
    }
}

/// The columns, in their one order (ADR 0068): To do, the Tasks not started, then the
/// Agents by state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum TasksColumn {
    ToDo,
    NeedsYou,
    Done,
    Working,
    Idle,
}

impl TasksColumn {
    /// The Agents' columns; To do holds Tasks and comes before them.
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

    /// To do's bit follows the Agents', which the GUI state file kept before it.
    pub(super) fn bit(self) -> u8 {
        match self {
            Self::NeedsYou => 1,
            Self::Done => 2,
            Self::Working => 4,
            Self::Idle => 8,
            Self::ToDo => 16,
        }
    }

    /// The colour of the column's ring, the one its cards' state shows in.
    fn tone(self) -> SidebarIconTone {
        match self {
            Self::NeedsYou => SidebarIconTone::Danger,
            Self::Done => SidebarIconTone::Success,
            Self::Working => SidebarIconTone::Warning,
            Self::ToDo | Self::Idle => SidebarIconTone::Muted,
        }
    }

    fn selector(self) -> &'static str {
        match self {
            Self::ToDo => "to-do",
            Self::NeedsYou => "needs-you",
            Self::Done => "done",
            Self::Working => "working",
            Self::Idle => "idle",
        }
    }

    fn label(self) -> SharedString {
        match self {
            Self::ToDo => t!("tasks.column.to_do"),
            Self::NeedsYou => t!("tasks.column.needs_you"),
            Self::Done => t!("tasks.column.done"),
            Self::Working => t!("tasks.column.working"),
            Self::Idle => t!("tasks.column.idle"),
        }
        .into_owned()
        .into()
    }
}

/// One Agent's card, or a started Task's while its Workspace has no Agent: where it
/// lives and how it shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TaskCard {
    pub(super) key: ConnectionKey,
    pub(super) workspace_id: WorkspaceId,
    /// The Agent's Pane, or for a Task's card its Workspace's focused Pane.
    pub(super) pane_id: PaneId,
    pub(super) status: SidebarStatusVisual,
    /// The Task whose card this is, which no Agent's card is.
    pub(super) task: Option<TaskId>,
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
        TypeId::of::<ToggleChanges>(),
        TypeId::of::<NextWorkspace>(),
        TypeId::of::<PreviousWorkspace>(),
    ]
    .contains(&action)
}

impl Condr {
    /// Whether `view` shows Tasks: the Tasks window, or the main window in Tasks mode
    /// while no Tasks window is open (ADR 0069).
    pub(super) fn in_tasks_mode(&self, view: &WindowView) -> bool {
        view.page == ContentPage::Tasks && view.handle == self.tasks_view().handle
    }

    /// Tasks has just come to `window`, or left a window for the main window: the form's
    /// inputs are made again where Tasks now is, and while `window` shows Tasks the
    /// keyboard goes back where the selection had it (ADR 0069).
    pub(super) fn settle_tasks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.rehome_task_form(window, cx);
        self.focus_tasks(window, cx);
    }

    /// The keyboard to the selected terminal when the selection had it, else to the
    /// card area, in `window` while it shows Tasks.
    pub(super) fn focus_tasks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.in_tasks_mode(self.window_view(window)) {
            return;
        }
        match self.tasks_view().tasks_selection {
            Some(selected) if selected.keyboard => {
                self.focus_tasks_terminal(selected.key, selected.pane_id, window, cx);
            }
            _ => self.tasks_view().tasks_focus.clone().focus(window, cx),
        }
        self.sync_terminal_focus(window, cx);
    }

    /// Enters or leaves Tasks mode. Neither changes the View (ADR 0021): leaving shows
    /// the Workspace and Tab shown before, and entering marks nothing seen.
    pub(super) fn toggle_tasks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.window_view(window);
        if view.scope != WindowScope::Main {
            return;
        }
        let page = if view.page == ContentPage::Tasks {
            ContentPage::Workspace
        } else {
            ContentPage::Tasks
        };
        self.show_page(page, window, cx);
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
            if matches!(
                self.windows[index].scope,
                WindowScope::Main | WindowScope::Tasks
            ) && self.windows[index].tasks_board.is_none()
            {
                let board = cx.new(|cx| TasksBoard::new(&owner, cx));
                self.windows[index].tasks_board = Some(board);
            }
        }
    }
}

/// The interaction area: selecting, the single-Pane Dock, the keyboard and the split.
impl Condr {
    /// Leaves the main window's Tasks mode, or the page saying Tasks is in its own window,
    /// for Go to's places and Open in Workspace.
    pub(super) fn leave_tasks_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.main_view().page == ContentPage::Tasks {
            self.main_view_mut().page = ContentPage::Workspace;
            self.main_view_mut().tasks_dock = None;
            if self.is_main(window) {
                self.rebuild_window_dock(window, cx);
            }
            self.schedule_state_save(cx);
            cx.notify();
        }
    }

    /// The Pane the keyboard goes to while Tasks shows: the selected card's, while its
    /// terminal holds the keyboard rather than the card area (ADR 0068).
    pub(super) fn tasks_keyboard_pane(&self) -> Option<(ConnectionKey, PaneId)> {
        self.tasks_view()
            .tasks_selection
            .filter(|selected| selected.keyboard)
            .map(|selected| (selected.key, selected.pane_id))
    }

    /// Whether the interaction area only previews `pane_id`: then the Client sends it no
    /// size, so stepping through cards resizes no Agent (ADR 0068).
    pub(super) fn tasks_previews(&self, key: ConnectionKey, pane_id: PaneId) -> bool {
        let view = self.tasks_view();
        view.page == ContentPage::Tasks
            && view.tasks_selection.is_some_and(|selected| {
                (selected.key, selected.pane_id) == (key, pane_id) && !selected.keyboard
            })
    }
}

/// The Device's To do Tasks in the Server's order: not started, archived ones never
/// being in the model.
pub(in crate::app) fn todo_tasks(connection: &ServerConnection) -> Vec<&TaskRecord> {
    let mut todo: Vec<&TaskRecord> = connection
        .model
        .tasks
        .iter()
        .flat_map(|tasks| tasks.values())
        .filter(|task| task.workspace.is_none())
        .collect();
    todo.sort_by_key(|task| (task.order, task.id));
    todo
}

/// The Device's repository Workspaces, which a Task's repository is one of: the root of
/// a Git repository, not a worktree made from one.
pub(in crate::app) fn repository_workspaces(
    connection: &ServerConnection,
) -> impl Iterator<Item = (WorkspaceId, String, PathBuf)> + '_ {
    connection
        .session()
        .into_iter()
        .flat_map(|session| session.workspaces())
        .filter(|workspace| {
            workspace.worktree().is_none()
                && connection
                    .model
                    .workspace_git
                    .get(&workspace.id())
                    .is_some_and(|git| git.root_repository().is_some())
        })
        .map(|workspace| {
            (
                workspace.id(),
                workspace.name().to_owned(),
                workspace.root_directory().to_path_buf(),
            )
        })
}

/// A repository by its Workspace's name, or its folder's when none is open.
pub(in crate::app) fn repository_name(
    connection: Option<&ServerConnection>,
    root: &Path,
) -> String {
    connection
        .and_then(|connection| connection.session()?.workspace_by_root(root))
        .map(|workspace| workspace.name().to_owned())
        .unwrap_or_else(|| folder_name(root))
}

/// The Pane of `workspace_id` this Client shows focused, as the Workspace would open.
pub(in crate::app) fn workspace_focused_pane(
    connection: &ServerConnection,
    session: &Session,
    workspace_id: WorkspaceId,
) -> Option<PaneId> {
    let tab = connection.viewed_tab_id(session, workspace_id)?;
    session.tab(tab)?.focused_pane().map(|pane| pane.id())
}

pub(in crate::app) fn workspace_has_agent(
    connection: &ServerConnection,
    workspace_id: WorkspaceId,
) -> bool {
    connection
        .session()
        .and_then(|session| session.workspace(workspace_id))
        .is_some_and(|workspace| {
            workspace
                .tabs()
                .iter()
                .flat_map(|tab| tab.panes())
                .any(|pane| connection.model.agents.contains_key(&pane.id()))
        })
}

#[cfg(test)]
mod tests {
    // Not `use super::*`: it would glob in `gpui_kit::test` and shadow the built-in attribute.
    use super::super::sidebar::{SidebarGlyph, SidebarIconTone};
    use super::super::{
        ClosePane, NextAgent, OpenSettings, SplitRight, ToggleSidebar, ToggleTasks,
    };
    use super::{
        ListFit, SidebarStatusVisual, TaskCard, TasksColumn, inert_in_tasks_mode, rank_column,
    };
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
    fn the_list_drops_branch_first_and_workspace_last() {
        assert_eq!(ListFit::within(80.), ListFit(4));
        assert_eq!(ListFit::within(ListFit::ALWAYS + 9.5 + 6.5), ListFit(2));
        assert_eq!(ListFit::within(20.), ListFit(0));
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
            task: None,
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
        assert!(!inert_in_tasks_mode(TypeId::of::<ToggleSidebar>()));
        assert!(!inert_in_tasks_mode(TypeId::of::<NextAgent>()));
        assert!(!inert_in_tasks_mode(TypeId::of::<OpenSettings>()));
        assert!(!inert_in_tasks_mode(TypeId::of::<ToggleTasks>()));
    }
}
