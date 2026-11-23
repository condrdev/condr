//! Ending a Task and putting it back: archiving, with the dialog a started one asks,
//! deleting, restoring, and moving one within To do (ADR 0068).

use super::*;
use condr_core::{TaskCommand, TaskId};
use gpui_kit::component::checkbox::Checkbox;
use rust_i18n::t;

/// What the archive dialog says about a started Task's Workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct ArchiveFacts {
    pub(in crate::app) title: String,
    /// A Start of this Task made the Workspace, as the Server recorded at its commit.
    pub(in crate::app) created: bool,
    pub(in crate::app) agents: usize,
    /// One Agent that waits for the person, else one that works.
    pub(in crate::app) busy: Option<(AgentKind, AgentDisplayState)>,
    pub(in crate::app) managed: bool,
    pub(in crate::app) dirty: bool,
    pub(in crate::app) branch: Option<String>,
}

pub(super) fn render_archive_choice(
    facts: &ArchiveFacts,
    choice: &Entity<(bool, bool)>,
    cx: &mut App,
) -> AnyElement {
    let (close, remove) = *choice.read(cx);
    let theme = cx.theme().clone();
    let muted = |text: String| {
        div()
            .pl(rems(1.5))
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(text)
    };
    let ending = if facts.agents == 1 {
        t!("tasks.archive.agents_end.one", count = facts.agents)
    } else {
        t!("tasks.archive.agents_end.other", count = facts.agents)
    };
    let busy = facts.busy.map(|(kind, state)| match state {
        AgentDisplayState::Blocked => t!("tasks.archive.waiting", agent = kind.label()),
        _ => t!("tasks.archive.working", agent = kind.label()),
    });
    let close_choice = choice.clone();
    let remove_choice = choice.clone();
    v_flex()
        .gap_2()
        .child(
            div().debug_selector(|| "task-archive-close".into()).child(
                Checkbox::new("task-archive-close")
                    .label(SharedString::from(t!("tasks.archive.close").into_owned()))
                    .checked(close)
                    .on_click(move |checked, _, cx| {
                        close_choice.update(cx, |choice, cx| {
                            choice.0 = *checked;
                            cx.notify();
                        });
                    }),
            ),
        )
        .child(
            h_flex()
                .gap_1()
                .child(muted(ending.into_owned()))
                .children(busy.map(|busy| {
                    div()
                        .text_sm()
                        .text_color(theme.warning)
                        .child(busy.into_owned())
                })),
        )
        .when(facts.managed, |this| {
            let label = match &facts.branch {
                Some(branch) => t!("tasks.archive.remove", branch = branch),
                None => t!("tasks.archive.remove_no_branch"),
            };
            this.child(
                div().debug_selector(|| "task-archive-remove".into()).child(
                    Checkbox::new("task-archive-remove")
                        .label(SharedString::from(label.into_owned()))
                        .checked(close && remove && !facts.dirty)
                        .disabled(!close || facts.dirty)
                        .on_click(move |checked, _, cx| {
                            remove_choice.update(cx, |choice, cx| {
                                choice.1 = *checked;
                                cx.notify();
                            });
                        }),
                ),
            )
            .when(facts.dirty, |this| {
                this.child(muted(t!("tasks.archive.dirty").into_owned()))
            })
        })
        .when(!close, |this| {
            this.child(muted(t!("tasks.archive.kept").into_owned()))
        })
        .into_any_element()
}

impl Condr {
    /// Archive on a To do Task's form, which asks nothing (ADR 0068).
    pub(super) fn archive_todo(
        &mut self,
        key: ConnectionKey,
        task: TaskId,
        cx: &mut Context<Self>,
    ) {
        self.task_form = None;
        self.send_task_change(
            key,
            TaskCommand::Archive {
                task,
                close_workspace: false,
                remove_worktree: false,
            },
            cx,
        );
    }

    /// Delete, from To do or Archived, after "Delete ‘…’? This can't be undone."
    pub(in crate::app) fn confirm_delete_task(
        &mut self,
        key: ConnectionKey,
        task: TaskId,
        title: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        super::dialogs::confirm_danger(
            cx.weak_entity(),
            super::dialogs::DangerConfirm::new(
                t!("tasks.delete.title", title = title),
                t!("tasks.delete.description"),
                t!("tasks.form.delete"),
            ),
            move |this, _, cx| {
                if this
                    .task_form
                    .as_ref()
                    .is_some_and(|form| form.home == Some((key, task)))
                {
                    this.task_form = None;
                }
                this.send_task_change(key, TaskCommand::Delete { task }, cx);
            },
            window,
            cx,
        );
    }

    pub(in crate::app) fn restore_task(
        &mut self,
        key: ConnectionKey,
        task: TaskId,
        cx: &mut Context<Self>,
    ) {
        self.send_task_change(key, TaskCommand::Restore { task }, cx);
    }

    /// Drops a To do Task where another of the same Device was: the Server counts the
    /// place among that Device's To do Tasks, the dragged one included.
    pub(in crate::app) fn reorder_todo(
        &mut self,
        key: ConnectionKey,
        task: TaskId,
        onto: TaskId,
        cx: &mut Context<Self>,
    ) {
        if task == onto {
            return;
        }
        let Some(index) = self.connection(key).and_then(|connection| {
            todo_tasks(connection)
                .iter()
                .position(|each| each.id == onto)
        }) else {
            return;
        };
        self.send_task_change(
            key,
            TaskCommand::Reorder {
                task,
                index: index as u32,
            },
            cx,
        );
    }

    /// Folds or unfolds To do's Archived list, fetching it from each Device as it opens.
    pub(in crate::app) fn toggle_archived_tasks(&mut self, cx: &mut Context<Self>) {
        self.tasks_archived_open = !self.tasks_archived_open;
        if self.tasks_archived_open {
            for connection in &mut self.connections {
                if connection.status == ConnectionStatus::Connected
                    && connection.model.tasks.is_some()
                {
                    connection.send_task(
                        TaskCommand::List { archived: true },
                        TaskRequest::ListArchived,
                    );
                }
            }
        }
        cx.notify();
    }

    /// Asks before archiving a started Task: whether to close its Workspace, checked when
    /// a Start of it made the Workspace, and whether to remove its worktree (ADR 0068).
    pub(in crate::app) fn confirm_archive_task(
        &mut self,
        key: ConnectionKey,
        task_id: TaskId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(facts) = self.archive_facts(key, task_id) else {
            return;
        };
        let choice = cx.new(|_| (facts.created, false));
        let error = cx.new(|_| None::<String>);
        let content = {
            let choice = choice.clone();
            let facts = facts.clone();
            move |_: &mut Window, cx: &mut App| render_archive_choice(&facts, &choice, cx)
        };
        let owner = cx.weak_entity();
        let submit = move |_: &mut Window, cx: &mut App| {
            let (close_workspace, remove_worktree) = *choice.read(cx);
            let command = TaskCommand::Archive {
                task: task_id,
                close_workspace,
                remove_worktree: close_workspace && remove_worktree,
            };
            let _ = owner.update(cx, |this, cx| this.send_task_change(key, command, cx));
            Ok::<(), String>(())
        };
        let title = t!("tasks.archive.title", title = facts.title).into_owned();
        window.defer(cx, move |window, cx| {
            super::dialogs::open_form_dialog(
                title.into(),
                t!("tasks.archive.ok").into(),
                error,
                content,
                submit,
                window,
                cx,
            );
        });
    }

    /// What the archive dialog says about a started Task's Workspace.
    pub(in crate::app) fn archive_facts(
        &self,
        key: ConnectionKey,
        task_id: TaskId,
    ) -> Option<ArchiveFacts> {
        let connection = self.connection(key)?;
        let task = connection.model.tasks.as_ref()?.get(&task_id)?;
        let workspace = connection.session()?.workspace(task.workspace?)?;
        let agents: Vec<(AgentKind, AgentDisplayState)> = workspace
            .tabs()
            .iter()
            .flat_map(|tab| tab.panes())
            .filter_map(|pane| {
                let agent = connection.model.agents.get(&pane.id())?;
                Some((agent.kind, connection.model.agent_display_state(pane.id())?))
            })
            .collect();
        let busy = agents
            .iter()
            .find(|(_, state)| matches!(state, AgentDisplayState::Blocked))
            .or_else(|| {
                agents
                    .iter()
                    .find(|(_, state)| matches!(state, AgentDisplayState::Working))
            })
            .copied();
        let managed = workspace
            .worktree()
            .is_some_and(|association| association.is_managed());
        let git = connection.model.workspace_git.get(&workspace.id());
        Some(ArchiveFacts {
            title: task.title.clone(),
            created: task.made_workspace,
            agents: agents.len(),
            busy,
            managed,
            dirty: git.is_some_and(|git| git.change_entries().next().is_some()),
            branch: git
                .and_then(WorkspaceGitSnapshot::root_repository)
                .and_then(|own| own.branch.clone()),
        })
    }
}
