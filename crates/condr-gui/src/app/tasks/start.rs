//! Starting a Task: where it may run, what Start would do, and the Start itself from the
//! request to the Agent taking it (ADR 0068).

use super::*;
use condr_core::{StartPlace, TaskCommand, TaskError, TaskId, TaskResponse, task_error};
use rust_i18n::t;
use std::path::Path;

/// A fully supported Agent (ADR 0035) whose CLI takes the prompt it starts on.
pub(in crate::app) fn starts_tasks(kind: AgentKind) -> bool {
    kind.spec().hooks().is_some() && kind.spec().initial_prompt.args("").is_some()
}

/// Where a Start runs, as the form offers it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum Where {
    Continue,
    Existing,
    NewWorktree,
}

/// Why Continue cannot be chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum ContinueBlock {
    /// Another Task holds the Workspace open on the folder; its title.
    Held(String),
    /// The Server said the folder is gone.
    Missing,
}

/// Why New worktree cannot be chosen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) enum WorktreeBlock {
    NoRepository,
    /// The repository's Workspace is not open; the repository's name.
    NotOpen(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct WorkspaceOption {
    pub(in crate::app) id: WorkspaceId,
    pub(in crate::app) name: String,
    /// The title of the other Task that holds it.
    pub(in crate::app) holder: Option<String>,
}

/// What Where offers a Task on its Device, and which it chooses by default (ADR 0068).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct WhereOptions {
    /// The last place's branch, or its folder's name, and why it is blocked.
    pub(in crate::app) continue_in: Option<(String, Option<ContinueBlock>)>,
    pub(in crate::app) workspaces: Vec<WorkspaceOption>,
    /// The Workspace Existing takes unless the person picks another.
    pub(in crate::app) existing: Option<WorkspaceId>,
    /// The repository Workspace's name.
    pub(in crate::app) new_worktree: Result<String, WorktreeBlock>,
    pub(in crate::app) default: Option<Where>,
}

impl WhereOptions {
    pub(in crate::app) fn allows(&self, place: Where) -> bool {
        match place {
            Where::Continue => matches!(self.continue_in, Some((_, None))),
            Where::Existing => self.workspaces.iter().any(|each| each.holder.is_none()),
            Where::NewWorktree => self.new_worktree.is_ok(),
        }
    }
}

/// The defaults' order: Continue where it was worked; else the repository's Workspace
/// when no Task holds it; else a new worktree of it; else the first free Workspace.
/// A folder the Server reported gone makes a new worktree the default.
pub(in crate::app) fn where_options(
    session: &Session,
    tasks: &BTreeMap<TaskId, TaskRecord>,
    task: &TaskRecord,
    folder_missing: bool,
) -> WhereOptions {
    let holder = |workspace_id: WorkspaceId| {
        tasks
            .values()
            .find(|other| other.id != task.id && other.workspace == Some(workspace_id))
            .map(|other| other.title.clone())
    };
    let continue_in = task.last.as_ref().map(|last| {
        let name = last
            .branch
            .clone()
            .unwrap_or_else(|| folder_name(&last.path));
        let block = if folder_missing {
            Some(ContinueBlock::Missing)
        } else {
            session
                .workspace_by_root(&last.path)
                .and_then(|workspace| holder(workspace.id()))
                .map(ContinueBlock::Held)
        };
        (name, block)
    });
    let workspaces: Vec<WorkspaceOption> = session
        .workspaces()
        .iter()
        .map(|workspace| WorkspaceOption {
            id: workspace.id(),
            name: workspace.name().to_owned(),
            holder: holder(workspace.id()),
        })
        .collect();
    let repository = task
        .repository
        .as_deref()
        .and_then(|root| session.workspace_by_root(root));
    let repository_free = repository.filter(|workspace| holder(workspace.id()).is_none());
    let first_free = workspaces
        .iter()
        .find(|each| each.holder.is_none())
        .map(|each| each.id);
    let new_worktree = match (&task.repository, repository) {
        (None, _) => Err(WorktreeBlock::NoRepository),
        (Some(root), None) => Err(WorktreeBlock::NotOpen(folder_name(root))),
        (Some(_), Some(workspace)) => Ok(workspace.name().to_owned()),
    };
    let default = match &continue_in {
        Some((_, None)) => Some(Where::Continue),
        Some((_, Some(ContinueBlock::Missing))) if new_worktree.is_ok() => Some(Where::NewWorktree),
        _ if repository_free.is_some() => Some(Where::Existing),
        _ if repository.is_some() => Some(Where::NewWorktree),
        _ => first_free.map(|_| Where::Existing),
    };
    WhereOptions {
        continue_in,
        workspaces,
        existing: repository_free.map(Workspace::id).or(first_free),
        new_worktree,
        default,
    }
}

pub(super) fn folder_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// What Start would do now; `blocked` says why it cannot.
pub(in crate::app) struct StartPlan {
    pub(in crate::app) options: WhereOptions,
    pub(in crate::app) place: Where,
    pub(in crate::app) workspace: Option<WorkspaceId>,
    pub(in crate::app) available: Option<Vec<AgentKind>>,
    pub(in crate::app) agent: Option<AgentKind>,
    pub(in crate::app) blocked: Option<String>,
}

/// The branch New worktree offers: the Server's rule for the title, or for a Task worked
/// before, the branch it had.
pub(super) fn default_branch(task: &TaskRecord) -> String {
    match task.repository.as_deref() {
        Some(repository) => condr_core::default_task_branch(&task.title, repository),
        None => condr_core::default_task_branch(&task.title, Path::new("")),
    }
}

impl Condr {
    /// Asks the Device which Agents can start, and how their hooks stand, unless known.
    pub(super) fn ask_task_agents(&mut self, key: ConnectionKey) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let asked = |request| connection.device_tasks.agent_requests.contains(&request);
        let ask_hooks = connection.hooks.is_empty()
            && !asked(AgentRequest::Hooks)
            && !asked(AgentRequest::InstallHooks);
        if connection.device_tasks.available.is_none() && !asked(AgentRequest::Available) {
            connection.send_agents_available();
        }
        if ask_hooks {
            self.request_agent_hooks(key);
        }
    }

    /// Selects a started Task's card: its Agent's, or while it has none, the Task's own
    /// on its Workspace's focused Pane.
    pub(in crate::app) fn select_started_task(
        &mut self,
        key: ConnectionKey,
        task_id: TaskId,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.connection(key) else {
            return;
        };
        let Some(workspace_id) = connection
            .model
            .tasks
            .as_ref()
            .and_then(|tasks| tasks.get(&task_id))
            .and_then(|task| task.workspace)
        else {
            return;
        };
        let Some(session) = connection.session() else {
            return;
        };
        let Some(workspace) = session.workspace(workspace_id) else {
            return;
        };
        let agent = workspace
            .tabs()
            .iter()
            .flat_map(|tab| tab.panes())
            .map(|pane| pane.id())
            .find(|pane_id| connection.model.agents.contains_key(pane_id));
        match agent {
            Some(pane_id) => self.select_card(key, pane_id, None, keyboard, window, cx),
            None => {
                let Some(pane_id) = workspace_focused_pane(connection, session, workspace_id)
                else {
                    return;
                };
                self.select_card(key, pane_id, Some(task_id), keyboard, window, cx);
            }
        }
    }

    /// An Agent left its Pane: when its Task has no Agent left, the selection on that
    /// Pane moves to the Task's card (ADR 0068).
    pub(in crate::app) fn agent_left_task(&mut self, key: ConnectionKey, pane_id: PaneId) {
        let Some(selected) = self
            .main_view()
            .tasks_selection
            .filter(|selected| (selected.key, selected.pane_id) == (key, pane_id))
        else {
            return;
        };
        let Some(connection) = self.connection(key) else {
            return;
        };
        let Some(workspace) = connection
            .session()
            .and_then(|session| session.workspace_for_pane(pane_id))
        else {
            return;
        };
        let Some(task) = connection.model.workspace_task(workspace.id()) else {
            return;
        };
        let agents_left = workspace
            .tabs()
            .iter()
            .flat_map(|tab| tab.panes())
            .any(|pane| connection.model.agents.contains_key(&pane.id()));
        if !agents_left {
            let task = task.id;
            if let Some(selection) = self.main_view_mut().tasks_selection.as_mut() {
                *selection = super::tasks::TasksSelection {
                    task: Some(task),
                    column: None,
                    ..selected
                };
            }
        }
    }

    /// Notes that a Task started, so its card reads "Starting…" until its Agent comes
    /// or `AGENT_WAIT` passes; for this Client's own Start, a toast says when none came.
    pub(super) fn watch_task_start(
        &mut self,
        key: ConnectionKey,
        task: TaskId,
        own: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        if let Some((_, mine)) = connection.device_tasks.waiting.get_mut(&task) {
            *mine |= own;
            return;
        }
        let started = Instant::now();
        connection.device_tasks.waiting.insert(task, (started, own));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(AGENT_WAIT).await;
            let _ = this.update(cx, |this, cx| {
                // Only this Start's wait: the Task may have gone back and started again.
                let current = this
                    .connection(key)
                    .and_then(|connection| connection.device_tasks.waiting.get(&task))
                    .map(|(at, _)| *at);
                if current == Some(started) {
                    this.task_start_settled(key, task, cx);
                }
            });
        })
        .detach();
    }

    /// `AGENT_WAIT` after a Start: the card reads "No agent running" if no Agent came,
    /// and for this Client's own Start one toast says so.
    pub(in crate::app) fn task_start_settled(
        &mut self,
        key: ConnectionKey,
        task_id: TaskId,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.connection_mut(key) else {
            return;
        };
        let Some((_, own)) = connection.device_tasks.waiting.remove(&task_id) else {
            return;
        };
        let no_agent = connection
            .model
            .tasks
            .as_ref()
            .and_then(|tasks| tasks.get(&task_id))
            .filter(|task| {
                task.workspace
                    .is_some_and(|workspace| !workspace_has_agent(connection, workspace))
            })
            .map(|task| task.title.clone());
        if own && let Some(title) = no_agent {
            self.report_error(t!("tasks.start.no_agent_started", title = title), cx);
        }
        cx.notify();
    }

    /// Whether a started Task's card still reads "Starting…".
    pub(in crate::app) fn task_starting(&self, key: ConnectionKey, task: TaskId) -> bool {
        self.connection(key).is_some_and(|connection| {
            connection
                .device_tasks
                .waiting
                .get(&task)
                .is_some_and(|(at, _)| at.elapsed() < AGENT_WAIT)
        })
    }

    /// Start: sends the Task, the place chosen and the Agent (ADR 0068).
    pub(super) fn start_task(&mut self, cx: &mut Context<Self>) {
        let Some(form) = &self.task_form else {
            return;
        };
        let Some((key, task_id)) = form.home else {
            return;
        };
        let Some(plan) = self.start_plan(form, cx) else {
            return;
        };
        let StartPlan {
            place,
            workspace,
            agent: Some(agent),
            blocked: None,
            ..
        } = plan
        else {
            return;
        };
        let place = match place {
            Where::Continue => StartPlace::Continue,
            Where::Existing => match workspace {
                Some(workspace) => StartPlace::Workspace(workspace),
                None => return,
            },
            Where::NewWorktree => StartPlace::NewWorktree {
                branch: Some(form.start.branch.read(cx).value().trim().to_owned())
                    .filter(|branch| !branch.is_empty()),
            },
        };
        self.last_task_agent = Some(agent);
        let request = self.connection_mut(key).and_then(|connection| {
            connection.send_task(
                TaskCommand::Start {
                    task: task_id,
                    place: Some(place),
                    agent,
                },
                TaskRequest::Start(task_id),
            )
        });
        if let Some(form) = &mut self.task_form {
            form.start.request = request;
            form.start.branch_error = None;
        }
        cx.notify();
    }

    pub(super) fn task_start_answered(
        &mut self,
        key: ConnectionKey,
        task_id: TaskId,
        result: Result<TaskResponse, TaskError>,
        cx: &mut Context<Self>,
    ) {
        if let Ok(TaskResponse::Started { pane_id, .. }) = result {
            self.watch_task_start(key, task_id, true, cx);
            // The started Task's card takes the area only from a form that loses nothing
            // by it: none, or this Task's own without changes.
            let follow = self
                .task_form
                .as_ref()
                .is_none_or(|form| form.home == Some((key, task_id)) && !form.changed(cx));
            if follow {
                self.task_form = None;
                self.in_main_window(cx, move |this, window, cx| {
                    if this.task_form.is_none() {
                        this.select_card(key, pane_id, Some(task_id), true, window, cx);
                    }
                });
            }
            return;
        }
        let form = self
            .task_form
            .as_mut()
            .filter(|form| form.home == Some((key, task_id)));
        match result {
            Ok(_) => {}
            Err(error) if error.code == task_error::BRANCH_CHECKED_OUT && form.is_some() => {
                let form = form.expect("checked above");
                form.start.request = None;
                form.start.branch_error = Some(error.message);
            }
            Err(error) if error.code == task_error::FOLDER_MISSING && form.is_some() => {
                let form = form.expect("checked above");
                form.start.request = None;
                form.start.folder_missing = true;
                form.start.place = None;
                let branch = self
                    .connection(key)
                    .and_then(|connection| connection.model.tasks.as_ref()?.get(&task_id))
                    .and_then(|task| task.last.as_ref()?.branch.clone());
                if let Some(branch) = branch {
                    let input = self
                        .task_form
                        .as_ref()
                        .map(|form| form.start.branch.clone());
                    self.in_main_window(cx, move |_, window, cx| {
                        if let Some(input) = input {
                            input.update(cx, |input, cx| input.set_value(branch, window, cx));
                        }
                    });
                }
            }
            Err(error) => {
                if let Some(form) = form {
                    form.start.request = None;
                }
                let title = self
                    .connection(key)
                    .and_then(|connection| connection.model.tasks.as_ref()?.get(&task_id))
                    .map(|task| task.title.clone())
                    .unwrap_or_default();
                self.report_error(
                    t!("tasks.start.failed", title = title, error = error.message),
                    cx,
                );
            }
        }
    }

    pub(super) fn choose_task_where(&mut self, place: Where, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.task_form {
            form.start.place = Some(place);
            cx.notify();
        }
    }

    pub(in crate::app) fn choose_task_workspace(
        &mut self,
        workspace: WorkspaceId,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = &mut self.task_form {
            form.start.place = Some(Where::Existing);
            form.start.workspace = Some(workspace);
            cx.notify();
        }
    }

    pub(super) fn choose_task_agent(&mut self, agent: AgentKind, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.task_form {
            form.start.agent = Some(agent);
            cx.notify();
        }
    }

    /// What Start would do now, and why it cannot when it cannot.
    pub(in crate::app) fn start_plan(&self, form: &TaskForm, cx: &App) -> Option<StartPlan> {
        let (key, task_id) = form.home?;
        let connection = self.connection(key)?;
        let tasks = connection.model.tasks.as_ref()?;
        let task = tasks.get(&task_id)?;
        let session = connection.session()?;
        let options = where_options(session, tasks, task, form.start.folder_missing);
        let place = form
            .start
            .place
            .filter(|place| options.allows(*place))
            .or(options.default);
        let workspace = form
            .start
            .workspace
            .filter(|workspace| {
                options
                    .workspaces
                    .iter()
                    .any(|each| each.id == *workspace && each.holder.is_none())
            })
            .or(options.existing);
        let available = connection.device_tasks.available.clone();
        let agent = available.as_ref().and_then(|available| {
            form.start
                .agent
                .or(self.last_task_agent)
                .filter(|agent| available.contains(agent))
                .or(available.first().copied())
        });
        let blocked = if form.changed(cx) {
            Some(t!("tasks.start.save_first").into_owned())
        } else if task.starting
            || task.workspace.is_some()
            || connection.device_tasks.asked(form.start.request)
        {
            Some(t!("tasks.todo.starting").into_owned())
        } else if place.is_none() {
            Some(t!("tasks.start.nowhere", device = connection.label).into_owned())
        } else {
            None
        };
        Some(StartPlan {
            options,
            place: place.unwrap_or(Where::Existing),
            workspace,
            available,
            agent,
            blocked,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{ContinueBlock, Where, WorktreeBlock, where_options};
    use condr_core::{LastPlace, Session, Task, TaskId, WorkspaceId};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn task(id: u128, repository: Option<&str>, workspace: Option<WorkspaceId>) -> Task {
        Task {
            id: TaskId::from_u128(id),
            title: format!("task {id}"),
            description: None,
            repository: repository.map(PathBuf::from),
            order: 0,
            archived: false,
            workspace,
            made_workspace: false,
            starting: false,
            last: None,
        }
    }

    fn tasks(list: &[&Task]) -> BTreeMap<TaskId, Task> {
        list.iter().map(|task| (task.id, (*task).clone())).collect()
    }

    #[test]
    fn where_defaults_follow_the_adr_order() {
        let mut session = Session::new();
        let repository = session.create_workspace("/src/api".into()).unwrap();
        let other = session.create_workspace("/src/web".into()).unwrap();

        // The repository's own Workspace, free: Existing takes it.
        let todo = task(1, Some("/src/api"), None);
        let options = where_options(&session, &tasks(&[&todo]), &todo, false);
        assert_eq!(options.default, Some(Where::Existing));
        assert_eq!(options.existing, Some(repository));
        assert_eq!(options.new_worktree, Ok("api".into()));
        assert_eq!(options.continue_in, None);

        // Another Task holds it: listed greyed with that Task's title, and a new
        // worktree is the default.
        let holder = task(2, None, Some(repository));
        let options = where_options(&session, &tasks(&[&todo, &holder]), &todo, false);
        assert_eq!(options.default, Some(Where::NewWorktree));
        assert_eq!(options.workspaces[0].holder.as_deref(), Some("task 2"));
        assert_eq!(options.existing, Some(other));

        // Worked before: Continue, unless the folder is gone, which makes a new
        // worktree the default, or another Task holds the Workspace open on it.
        let mut worked = todo.clone();
        worked.last = Some(LastPlace {
            path: "/src/web".into(),
            branch: Some("worktree/fix".into()),
            base_branch: None,
            managed: true,
            repository: Some("/src/api".into()),
        });
        let options = where_options(&session, &tasks(&[&worked]), &worked, false);
        assert_eq!(options.default, Some(Where::Continue));
        assert_eq!(options.continue_in, Some(("worktree/fix".into(), None)));
        let options = where_options(&session, &tasks(&[&worked]), &worked, true);
        assert_eq!(
            options.continue_in,
            Some(("worktree/fix".into(), Some(ContinueBlock::Missing)))
        );
        assert_eq!(options.default, Some(Where::NewWorktree));
        let web_holder = task(3, None, Some(other));
        let options = where_options(&session, &tasks(&[&worked, &web_holder]), &worked, false);
        assert_eq!(
            options.continue_in.unwrap().1,
            Some(ContinueBlock::Held("task 3".into()))
        );

        // No repository: the first free Workspace, and no new worktree.
        let loose = task(4, None, None);
        let options = where_options(&session, &tasks(&[&loose, &holder]), &loose, false);
        assert_eq!(options.default, Some(Where::Existing));
        assert_eq!(options.existing, Some(other));
        assert_eq!(options.new_worktree, Err(WorktreeBlock::NoRepository));

        // A repository not open, and every Workspace held: nothing to offer.
        let closed = task(5, Some("/src/cli"), None);
        let all_held = task(6, None, Some(other));
        let options = where_options(
            &session,
            &tasks(&[&closed, &holder, &all_held]),
            &closed,
            false,
        );
        assert_eq!(
            options.new_worktree,
            Err(WorktreeBlock::NotOpen("cli".into()))
        );
        assert_eq!(options.default, None);
        assert!(!options.allows(Where::Existing));
    }
}
