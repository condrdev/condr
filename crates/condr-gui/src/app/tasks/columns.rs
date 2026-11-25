//! Which cards Tasks mode shows and in which column: the scope, the Agents' columns by
//! state and urgency, To do's Tasks, and folding (ADR 0068).

use super::*;

impl Condr {
    pub(super) fn toggle_tasks_column(&mut self, column: TasksColumn, cx: &mut Context<Self>) {
        self.tasks_view_mut().tasks.folded ^= column.bit();
        self.schedule_state_save(cx);
        cx.notify();
    }

    pub(in crate::app) fn set_tasks_scope(&mut self, scope: TasksScope, cx: &mut Context<Self>) {
        self.tasks_view_mut().tasks.scope = scope;
        self.schedule_state_save(cx);
        cx.notify();
    }

    /// Unfolds a folded Needs you when an Agent that was not in it arrives; the other
    /// columns only count theirs (ADR 0068).
    pub(in crate::app) fn unfold_needs_you_for_arrivals(&mut self, cx: &mut Context<Self>) {
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
        let tasks = &mut self.tasks_view_mut().tasks;
        if arrived && tasks.is_folded(TasksColumn::NeedsYou) {
            tasks.folded &= !TasksColumn::NeedsYou.bit();
            self.schedule_state_save(cx);
        }
    }

    /// The scope to show: a Workspace a connected Device no longer has, or a Device no
    /// longer configured, falls back to All. A Device that is only disconnected keeps it.
    pub(in crate::app) fn effective_tasks_scope(&self, scope: TasksScope) -> TasksScope {
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
    pub(super) fn tasks_scope_shows(
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

    /// Every Agent on a connected Device in `scope`, by column, each column by urgency;
    /// the selected card's too, so it moves with its state (ADR 0075).
    pub(in crate::app) fn task_cards(
        &self,
        scope: TasksScope,
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
                let mut cards = Vec::new();
                for pane in workspace.tabs().iter().flat_map(|tab| tab.panes()) {
                    let Some(status) = pane_sidebar_status(&connection.model, pane.id()) else {
                        continue;
                    };
                    cards.push(TaskCard {
                        key: connection.key,
                        workspace_id: workspace.id(),
                        pane_id: pane.id(),
                        status,
                        task: None,
                    });
                }
                // A started Task whose Workspace has no Agent is a card of its own, so it
                // never leaves Tasks mode (ADR 0068).
                if cards.is_empty()
                    && let Some(task) = connection.model.workspace_task(workspace.id())
                    && let Some(pane_id) =
                        workspace_focused_pane(connection, session, workspace.id())
                {
                    let state = if self.task_starting(connection.key, task.id) {
                        AgentDisplayState::Working
                    } else {
                        AgentDisplayState::Idle
                    };
                    cards.push(TaskCard {
                        key: connection.key,
                        workspace_id: workspace.id(),
                        pane_id,
                        status: super::sidebar::agent_sidebar_status(state),
                        task: Some(task.id),
                    });
                }
                for card in cards {
                    let column = TasksColumn::of(card.status);
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

    /// The To do Tasks of the connected Devices in `scope`, each Device's in its order. A
    /// Task shows under its repository's Workspace, and one without a repository only
    /// under All or its Device.
    pub(in crate::app) fn todo_cards(&self, scope: TasksScope) -> Vec<(ConnectionKey, TaskRecord)> {
        let mut cards = Vec::new();
        for connection in self.task_devices() {
            let shown = |task: &TaskRecord| match scope {
                TasksScope::All => true,
                TasksScope::Device(server) => connection.model.server_id == Some(server),
                TasksScope::Workspace(server, workspace_id) => {
                    connection.model.server_id == Some(server)
                        && task.repository.as_deref().is_some_and(|root| {
                            connection
                                .session()
                                .and_then(|session| session.workspace_by_root(root))
                                .is_some_and(|workspace| workspace.id() == workspace_id)
                        })
                }
            };
            cards.extend(
                todo_tasks(connection)
                    .into_iter()
                    .filter(|task| shown(task))
                    .map(|task| (connection.key, task.clone())),
            );
        }
        cards
    }

    /// Whether To do shows: some Device in `scope` keeps Tasks.
    pub(super) fn offers_todo(&self, scope: TasksScope) -> bool {
        self.task_devices().any(|connection| match scope {
            TasksScope::All => true,
            TasksScope::Device(server) | TasksScope::Workspace(server, _) => {
                connection.model.server_id == Some(server)
            }
        })
    }
}
