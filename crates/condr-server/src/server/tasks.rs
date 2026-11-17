//! The Device's Tasks (ADR 0068): kept in the Server's state under its lock, written to
//! the task store off it, and sent only to the Clients that said `tasks` in `Hello`. A
//! started Task holds one Workspace; the Server clears that link in the same step the
//! Workspace closes.

use super::*;
use condr_core::{LastPlace, Task, TaskCommand, TaskError, TaskId, TaskResponse};
use condr_core::{MAX_TASK_DESCRIPTION_BYTES, MAX_TASK_TITLE_BYTES, task_error};
use std::collections::{BTreeMap, HashMap};

pub(super) struct Tasks {
    /// Every Task, archived ones included, each with its description.
    items: BTreeMap<TaskId, Task>,
    #[cfg(feature = "tasks")]
    store: Option<crate::task_store::TaskStore>,
    /// Where each Task's file goes once it starts: `<data directory>/tasks`.
    files: PathBuf,
    /// The Workspace each Start under way has chosen, which no other Start may take.
    pub(super) claims: HashMap<TaskId, WorkspaceId>,
    /// The agents a test starts in place of the ones on PATH.
    #[cfg(test)]
    pub(super) agents: Option<Vec<condr_core::agent_discovery::AgentInstallation>>,
    /// Makes every Pane's shell look busy to a Start, which a test stops the Server under.
    #[cfg(test)]
    pub(super) shells_busy: bool,
}

impl Tasks {
    /// Tasks kept in memory only, their files under `files`.
    #[cfg_attr(not(feature = "tasks"), allow(dead_code))]
    pub(super) fn in_memory(files: PathBuf) -> Self {
        Self {
            items: BTreeMap::new(),
            #[cfg(feature = "tasks")]
            store: None,
            files,
            claims: HashMap::new(),
            #[cfg(test)]
            agents: None,
            #[cfg(test)]
            shells_busy: false,
        }
    }

    /// The Device's Tasks from `tasks.sqlite` in `data_directory`, or in memory for a
    /// Server without one; `None` for a build without the task store, which keeps none.
    pub(super) fn open(data_directory: Option<&std::path::Path>, files: PathBuf) -> Option<Self> {
        #[cfg(not(feature = "tasks"))]
        {
            let _ = (data_directory, files);
            None
        }
        #[cfg(feature = "tasks")]
        {
            let mut tasks = Self::in_memory(files);
            let Some(directory) = data_directory else {
                return Some(tasks);
            };
            let path = directory.join("tasks.sqlite");
            match crate::task_store::TaskStore::open(&path) {
                Ok(opened) => {
                    tasks.items = opened
                        .tasks
                        .into_iter()
                        .map(|task| (task.id, task))
                        .collect();
                    tasks.store = Some(opened.store);
                }
                Err(error) => tracing::error!(
                    "cannot open the task store {}: {error}; Tasks are kept in memory until \
                     the Server restarts",
                    path.display()
                ),
            }
            Some(tasks)
        }
    }

    /// Writes what is pending; the Server calls it on its way out.
    pub(super) fn shutdown(&mut self) -> io::Result<()> {
        #[cfg(feature = "tasks")]
        if let Some(store) = &mut self.store {
            return store.shutdown();
        }
        Ok(())
    }

    fn persist(&self, id: TaskId) {
        #[cfg(feature = "tasks")]
        if let Some(store) = &self.store {
            store.schedule(id, self.items.get(&id).cloned());
        }
        #[cfg(not(feature = "tasks"))]
        let _ = id;
    }

    pub(super) fn file(&self, id: TaskId) -> PathBuf {
        self.files.join(format!("{id}.md"))
    }

    /// The Task that holds `workspace_id`, or a Start under way that chose it.
    pub(super) fn holder(&self, workspace_id: WorkspaceId) -> Option<TaskId> {
        self.items
            .values()
            .find(|task| task.workspace == Some(workspace_id))
            .map(|task| task.id)
            .or_else(|| {
                self.claims
                    .iter()
                    .find(|(_, claimed)| **claimed == workspace_id)
                    .map(|(id, _)| *id)
            })
    }

    /// Above every To do and started Task.
    fn top(&self) -> i64 {
        self.items
            .values()
            .filter(|task| !task.archived)
            .map(|task| task.order)
            .min()
            .map_or(0, |order| order - 1)
    }

    /// The To do and started Tasks, or the archived ones, in order and without
    /// descriptions.
    pub(super) fn summaries(&self, archived: bool) -> Vec<Task> {
        let mut tasks = self
            .items
            .values()
            .filter(|task| task.archived == archived)
            .map(Task::summary)
            .collect::<Vec<_>>();
        tasks.sort_by_key(|task| (task.order, task.id));
        tasks
    }
}

fn not_found(id: TaskId) -> TaskError {
    TaskError::new(
        task_error::TASK_NOT_FOUND,
        format!("no Task {id} on this Device"),
    )
}

fn invalid(message: impl Into<String>) -> TaskError {
    TaskError::new(task_error::INVALID_TASK, message)
}

/// A title as kept: trimmed, one line, and not too long.
fn valid_title(title: &str) -> Result<String, TaskError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(invalid("a Task needs a title"));
    }
    if title.len() > MAX_TASK_TITLE_BYTES {
        return Err(invalid(format!(
            "a title is at most {MAX_TASK_TITLE_BYTES} bytes"
        )));
    }
    if title.chars().any(char::is_control) {
        return Err(invalid("a title is one line of text"));
    }
    Ok(title.to_owned())
}

fn valid_description(description: &str) -> Result<(), TaskError> {
    if description.len() > MAX_TASK_DESCRIPTION_BYTES {
        return Err(invalid("a description is at most 64 KiB"));
    }
    Ok(())
}

impl RuntimeState {
    fn tasks_mut(&mut self) -> &mut Tasks {
        self.tasks
            .as_mut()
            .expect("Task requests reach only a Server that keeps Tasks")
    }

    /// The Task `id`; once a stopping Server has taken its Tasks, none.
    pub(super) fn task_mut(&mut self, id: TaskId) -> Result<&mut Task, TaskError> {
        self.tasks
            .as_mut()
            .ok_or_else(stopping_server)?
            .items
            .get_mut(&id)
            .ok_or_else(|| not_found(id))
    }

    /// Stores `id` and tells the Clients that take Tasks.
    pub(super) fn task_changed(&mut self, id: TaskId) {
        let Some(tasks) = &self.tasks else { return };
        tasks.persist(id);
        if let Some(task) = tasks.items.get(&id) {
            let event = SessionEvent::TaskChanged(task.summary());
            self.publish_background(event);
        }
    }

    /// Where `workspace_id` has its work, for a Task to continue there (ADR 0068).
    pub(super) fn last_place(&self, workspace_id: WorkspaceId) -> Option<LastPlace> {
        let workspace = self.session.workspace(workspace_id)?;
        let association = workspace.worktree();
        Some(LastPlace {
            path: workspace.root_directory().to_path_buf(),
            branch: self
                .workspace_git
                .get(&workspace_id)
                .and_then(|git| git.repository.branch().map(str::to_owned)),
            base_branch: association
                .and_then(|association| association.base_branch().map(str::to_owned)),
            managed: association.is_some_and(|association| association.is_managed()),
            repository: association
                .map(|association| association.parent_root_directory().to_path_buf()),
        })
    }

    /// Lets go of every Workspace `next` no longer has, recording where its Task was
    /// worked from the structure that still has it. Called just before `next` becomes the
    /// Session, so a Workspace number used again never points at an old Task; the caller
    /// publishes the returned Tasks once it has.
    pub(super) fn release_closed_task_workspaces(&mut self, next: &Session) -> Vec<TaskId> {
        let Some(tasks) = &self.tasks else {
            return Vec::new();
        };
        let lost = tasks
            .items
            .values()
            .filter_map(|task| {
                let workspace_id = task.workspace?;
                next.workspace(workspace_id)
                    .is_none()
                    .then_some((task.id, workspace_id))
            })
            .collect::<Vec<_>>();
        for &(id, workspace_id) in &lost {
            let last = self.last_place(workspace_id);
            let task = self.tasks_mut().items.get_mut(&id).expect("listed above");
            task.workspace = None;
            task.made_workspace = false;
            if last.is_some() {
                task.last = last;
            }
            tracing::info!(
                task = %id,
                workspace = workspace_id.as_u64(),
                "a Task's Workspace closed; it is back in To do"
            );
        }
        lost.into_iter().map(|(id, _)| id).collect()
    }

    /// After a restart: a Task whose Workspace did not come back keeps the last place its
    /// Start recorded and returns to To do (ADR 0068).
    pub(super) fn release_lost_task_workspaces(&mut self) {
        let session = self.session.clone();
        for id in self.release_closed_task_workspaces(&session) {
            self.task_changed(id);
        }
    }

    /// The Tasks a Bootstrap carries to a Client that said `tasks`.
    pub(super) fn bootstrap_tasks(&self, hinted: bool) -> Option<Vec<Task>> {
        hinted
            .then(|| self.tasks.as_ref().map(|tasks| tasks.summaries(false)))
            .flatten()
    }

    /// Answers every Task command but Start and an Archive that closes its Workspace,
    /// which take their time off the lock.
    pub(super) fn task_command(&mut self, command: TaskCommand) -> Result<TaskResponse, TaskError> {
        if self.tasks.is_none() {
            return Err(stopping_server());
        }
        match command {
            TaskCommand::Create {
                title,
                description,
                repository,
            } => {
                let title = valid_title(&title)?;
                valid_description(&description)?;
                if let Some(repository) = &repository {
                    self.valid_repository(repository)?;
                }
                let tasks = self.tasks_mut();
                let id = TaskId::from_uuid(uuid::Uuid::now_v7());
                let task = Task {
                    id,
                    title,
                    description: Some(description),
                    repository,
                    order: tasks.top(),
                    archived: false,
                    workspace: None,
                    made_workspace: false,
                    starting: false,
                    last: None,
                };
                tasks.items.insert(id, task.clone());
                self.task_changed(id);
                Ok(TaskResponse::Task(task))
            }
            TaskCommand::Edit {
                task: id,
                title,
                description,
                repository,
            } => {
                let title = title.as_deref().map(valid_title).transpose()?;
                if let Some(description) = &description {
                    valid_description(description)?;
                }
                if let Some(Some(repository)) = &repository {
                    self.valid_repository(repository)?;
                }
                let task = self.task_mut(id)?;
                if let Some(title) = title {
                    task.title = title;
                }
                if let Some(description) = description {
                    task.description = Some(description);
                }
                if let Some(repository) = repository {
                    task.repository = repository;
                }
                let task = task.clone();
                self.task_changed(id);
                Ok(TaskResponse::Task(task))
            }
            TaskCommand::Delete { task: id } => {
                let task = self.task_mut(id)?;
                refuse_busy(task)?;
                self.tasks_mut().items.remove(&id);
                self.tasks_mut().persist(id);
                self.publish_background(SessionEvent::TaskRemoved { task: id });
                Ok(TaskResponse::Done)
            }
            TaskCommand::Archive { task: id, .. } => {
                if self.task_mut(id)?.starting {
                    return Err(starting(id));
                }
                Ok(TaskResponse::Task(self.archive(id)))
            }
            TaskCommand::Restore { task: id } => {
                let top = self.tasks_mut().top();
                let task = self.task_mut(id)?;
                if task.archived {
                    task.archived = false;
                    task.order = top;
                    self.task_changed(id);
                }
                Ok(TaskResponse::Task(self.task_mut(id)?.clone()))
            }
            TaskCommand::Reorder { task: id, index } => {
                let task = self.task_mut(id)?;
                if task.archived {
                    return Err(TaskError::new(
                        task_error::TASK_ARCHIVED,
                        "only a Task in To do moves",
                    ));
                }
                refuse_busy(task)?;
                self.reorder(id, index as usize);
                Ok(TaskResponse::Done)
            }
            TaskCommand::Get { task: id } => Ok(TaskResponse::Task(self.task_mut(id)?.clone())),
            TaskCommand::List { archived } => {
                Ok(TaskResponse::Tasks(self.tasks_mut().summaries(archived)))
            }
            TaskCommand::Start { .. } => Err(invalid("a Start is answered off the lock")),
        }
    }

    /// Archives `id`, letting go of its Workspace, which stays open (ADR 0068).
    pub(super) fn archive(&mut self, id: TaskId) -> Task {
        let workspace = self.tasks_mut().items[&id].workspace;
        let last = workspace.and_then(|workspace| self.last_place(workspace));
        let task = self
            .tasks_mut()
            .items
            .get_mut(&id)
            .expect("checked by the caller");
        task.archived = true;
        task.workspace = None;
        task.made_workspace = false;
        if last.is_some() {
            task.last = last;
        }
        let task = task.clone();
        self.task_changed(id);
        task
    }

    /// Moves a To do Task to `index` among the To do Tasks and numbers them again.
    fn reorder(&mut self, id: TaskId, index: usize) {
        let tasks = self.tasks_mut();
        let mut todo = tasks
            .items
            .values()
            .filter(|task| !task.archived && task.workspace.is_none() && task.id != id)
            .map(|task| (task.order, task.id))
            .collect::<Vec<_>>();
        todo.sort();
        let mut order = todo.into_iter().map(|(_, id)| id).collect::<Vec<_>>();
        order.insert(index.min(order.len()), id);
        let changed = order
            .into_iter()
            .zip(0..)
            .filter_map(|(id, position)| {
                let task = tasks.items.get_mut(&id)?;
                (task.order != position).then(|| {
                    task.order = position;
                    id
                })
            })
            .collect::<Vec<_>>();
        for id in changed {
            self.task_changed(id);
        }
    }

    /// A Task's repository: the root of a repository Workspace open on this Device.
    fn valid_repository(&self, path: &std::path::Path) -> Result<(), TaskError> {
        let open = path.is_absolute()
            && self
                .session
                .workspace_by_root(path)
                .is_some_and(|workspace| self.workspace_git.contains_key(&workspace.id()));
        if open {
            Ok(())
        } else {
            Err(invalid(format!(
                "no repository Workspace is open at {}",
                path.display()
            )))
        }
    }
}

fn stopping_server() -> TaskError {
    TaskError::new(task_error::SERVER_STOPPING, "the Server is stopping")
}

fn starting(id: TaskId) -> TaskError {
    TaskError::new(task_error::TASK_STARTING, format!("Task {id} is starting"))
}

/// Refuses a Task that is starting or started, which only archiving ends.
fn refuse_busy(task: &Task) -> Result<(), TaskError> {
    if task.starting {
        return Err(starting(task.id));
    }
    if task.workspace.is_some() {
        return Err(TaskError::new(
            task_error::TASK_STARTED,
            format!("Task {} is started; archive it first", task.id),
        ));
    }
    Ok(())
}

pub(super) fn reply_task(
    outbound: &ClientWriter,
    request_id: u64,
    result: Result<TaskResponse, TaskError>,
) -> bool {
    let message = ServerMessage::Task { request_id, result };
    match frame_message(&message) {
        Ok(data) => outbound.send_reliable(data) == Err(ReliableSendError::Disconnected),
        Err(error) => queue_message(
            outbound,
            ServerMessage::Task {
                request_id,
                result: Err(invalid(format!("the answer cannot be sent: {error}"))),
            },
        ),
    }
}

/// One `ClientMessage::Task`. Says whether the connection must close.
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_task_request(
    state: &Arc<Mutex<RuntimeState>>,
    lifecycle: &Arc<ServerLifecycle>,
    layouts: &ClientLayouts,
    outbound: &ClientWriter,
    server_id: ServerId,
    request_id: u64,
    command: TaskCommand,
) -> bool {
    {
        let state = state.lock().or_abort();
        // A build without the store answers as a Server that predates Tasks.
        if state.tasks.is_none() {
            return queue_message(
                outbound,
                ServerMessage::Error {
                    message: condr_core::protocol::UNSUPPORTED_REQUEST.into(),
                },
            );
        }
        if server_id != state.server_id {
            return reply_task(outbound, request_id, Err(invalid("unknown Server")));
        }
    }
    match command {
        TaskCommand::Start { task, place, agent } => {
            let (state, lifecycle, outbound) =
                (Arc::clone(state), Arc::clone(lifecycle), outbound.clone());
            thread::spawn(move || {
                let result = super::task_start::start(&state, &lifecycle, task, place, agent);
                reply_task(&outbound, request_id, result);
            });
            false
        }
        TaskCommand::Archive {
            task,
            close_workspace: true,
            remove_worktree,
        } => {
            let (state, lifecycle, layouts, outbound) = (
                Arc::clone(state),
                Arc::clone(lifecycle),
                layouts.clone(),
                outbound.clone(),
            );
            thread::spawn(move || {
                archive_and_close(
                    &state,
                    &lifecycle,
                    &layouts,
                    &outbound,
                    request_id,
                    task,
                    remove_worktree,
                )
            });
            false
        }
        command => {
            let removed = match &command {
                TaskCommand::Delete { task } | TaskCommand::Archive { task, .. } => Some(*task),
                _ => None,
            };
            let (result, file) = {
                let mut state = state.lock().or_abort();
                let result = state.task_command(command);
                let file = removed.and_then(|task| Some(state.tasks.as_ref()?.file(task)));
                (result, file)
            };
            if result.is_ok()
                && let Some(file) = file
            {
                remove_task_file(&file);
            }
            reply_task(outbound, request_id, result)
        }
    }
}

/// An archived or deleted Task's file goes with it (ADR 0068).
fn remove_task_file(file: &std::path::Path) {
    if let Err(error) = std::fs::remove_file(file)
        && error.kind() != io::ErrorKind::NotFound
    {
        tracing::warn!("left the task file {} behind: {error}", file.display());
    }
}

/// Archives a started Task and then closes its Workspace, removing a Managed Worktree
/// when asked. The answer goes first, since the requester may run in that Workspace.
/// Shutdown waits for it (ADR 0053), and a panic answers with an error.
pub(super) fn archive_and_close(
    state: &Arc<Mutex<RuntimeState>>,
    lifecycle: &Arc<ServerLifecycle>,
    layouts: &ClientLayouts,
    outbound: &ClientWriter,
    request_id: u64,
    id: TaskId,
    remove_worktree: bool,
) {
    let Some(_operation) = lifecycle.begin_operation() else {
        reply_task(outbound, request_id, Err(stopping_server()));
        return;
    };
    let archived = answer_or_fail(
        || archive_for_close(state, id, remove_worktree),
        |message| Err(TaskError::new(task_error::INTERNAL_ERROR, message)),
    );
    let (task, close, file) = match archived {
        Ok(archived) => archived,
        Err(error) => {
            reply_task(outbound, request_id, Err(error));
            return;
        }
    };
    remove_task_file(&file);
    reply_task(outbound, request_id, Ok(TaskResponse::Task(task)));
    if let Some(command) = close {
        let closed = answer_or_fail(|| layouts.apply(command), Err);
        if let Err(error) = closed {
            tracing::warn!(
                task = %id,
                "the archived Task's Workspace stays open: {error}"
            );
        }
    }
}

/// Archives `id` once its worktree, when it is to go, may be removed; returns the Task,
/// how to close its Workspace, and its file.
fn archive_for_close(
    state: &Arc<Mutex<RuntimeState>>,
    id: TaskId,
    remove_worktree: bool,
) -> Result<(Task, Option<LayoutCommand>, PathBuf), TaskError> {
    let planned = {
        let mut state = state.lock().or_abort();
        state
            .task_mut(id)
            .and_then(|task| {
                if task.starting {
                    return Err(starting(id));
                }
                Ok(task.workspace)
            })
            .and_then(|workspace| {
                let removal = match workspace {
                    Some(workspace_id) if remove_worktree => {
                        let workspace = state.session.workspace(workspace_id);
                        let association = workspace
                            .and_then(|workspace| workspace.worktree())
                            .filter(|association| association.is_managed())
                            .ok_or_else(|| {
                                TaskError::new(
                                    task_error::WORKTREE_NOT_REMOVABLE,
                                    "only a Managed Worktree is removed",
                                )
                            })?;
                        Some((
                            association.parent_root_directory().to_path_buf(),
                            workspace
                                .expect("has an association")
                                .root_directory()
                                .to_path_buf(),
                        ))
                    }
                    _ => None,
                };
                Ok((workspace, removal))
            })
    };
    let (workspace, removal) = planned?;
    // A worktree with changes stays, and so does its Task (ADR 0002).
    if let Some((parent_root, root)) = &removal {
        let checked = discover_repository(parent_root)
            .and_then(|parent| Ok((parent, discover_repository(root)?)))
            .map_err(|error| error.to_string())
            .and_then(|found| match found {
                (Some(parent), Some(child)) => {
                    validate_worktree_removal(&parent, &child).map_err(|error| error.to_string())
                }
                _ => Err("the Workspace is not a Git worktree".into()),
            });
        checked.map_err(|message| TaskError::new(task_error::WORKTREE_NOT_REMOVABLE, message))?;
    }
    let mut state = state.lock().or_abort();
    if state.task_mut(id)?.starting {
        return Err(starting(id));
    }
    let file = state.tasks_mut().file(id);
    let task = state.archive(id);
    let close = workspace.map(|workspace_id| {
        if removal.is_some() {
            LayoutCommand::RemoveWorktree { workspace_id }
        } else {
            LayoutCommand::CloseWorkspace { workspace_id }
        }
    });
    Ok((task, close, file))
}
