//! `StartTask` (ADR 0068): reserve the Task, prepare its Workspace, write its file, wait
//! for the shell and type the Agent's command, then link. Typing is the commit point: a
//! failure before it undoes only what this Start made, and nothing after it is undone.

use super::*;
use condr_core::agent_discovery::AgentInstallation;
use condr_core::{
    AgentKind, StartPlace, TabId, TaskError, TaskId, TaskResponse, branch_is_checked_out,
    default_task_branch, delete_branch_at, task_error,
};

/// How long a new Pane's shell has to reach its prompt.
const SHELL_READY_TIMEOUT: Duration = Duration::from_secs(30);
/// What an npm `.cmd` shim refuses in an argument (ADR 0068).
const SHIM_REFUSES: [char; 10] = ['&', '|', '<', '>', '^', '(', ')', '%', '!', '"'];

/// Where the Start runs, as resolved under the lock.
enum Plan {
    /// A new Tab in an open Workspace.
    Existing(WorkspaceId),
    /// The Task's last folder opened again; `managed` re-associates it with the
    /// repository's open Workspace when the last place says Condr made it.
    Reopen {
        path: PathBuf,
        managed: Option<(WorkspaceId, PathBuf, Option<String>)>,
    },
    NewWorktree {
        parent_workspace_id: WorkspaceId,
        parent_root: PathBuf,
        branch: String,
    },
}

/// What this Start made, which a failure before the commit point undoes.
#[derive(Default)]
pub(super) struct Made {
    pub(super) workspace: Option<WorkspaceId>,
    pub(super) tab: Option<TabId>,
    /// The repository and the worktree created in it.
    pub(super) worktree: Option<(GitRepository, GitRepository)>,
    /// A branch created for the worktree, with the commit it was made at.
    pub(super) branch: Option<(GitRepository, String, String)>,
}

/// Holds the Task in "Starting" and the Workspace it chose; dropping it before the
/// commit, by a failure or a panic, gives both back.
struct Reservation<'a> {
    state: &'a Arc<Mutex<RuntimeState>>,
    task: TaskId,
    committed: bool,
}

impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        let mut state = self.state.lock().or_abort();
        if let Some(tasks) = state.tasks.as_mut() {
            tasks.claims.remove(&self.task);
        }
        if let Ok(task) = state.task_mut(self.task) {
            task.starting = false;
            state.task_changed(self.task);
        }
    }
}

fn error(code: &str, message: impl Into<String>) -> TaskError {
    TaskError::new(code, message)
}

/// Runs one Start to its answer. It belongs to the Server, not to the connection that
/// asked: a requester that leaves does not stop it (ADR 0068).
pub(super) fn start(
    state: &Arc<Mutex<RuntimeState>>,
    lifecycle: &Arc<ServerLifecycle>,
    task: TaskId,
    place: Option<StartPlace>,
    agent: AgentKind,
) -> Result<TaskResponse, TaskError> {
    let Some(_operation) = lifecycle.begin_operation() else {
        return Err(error(task_error::SERVER_STOPPING, "the Server is stopping"));
    };
    answer_or_fail(
        || run(state, lifecycle, task, place, agent),
        |message| Err(error(task_error::INTERNAL_ERROR, message)),
    )
}

fn run(
    state: &Arc<Mutex<RuntimeState>>,
    lifecycle: &Arc<ServerLifecycle>,
    id: TaskId,
    place: Option<StartPlace>,
    agent: AgentKind,
) -> Result<TaskResponse, TaskError> {
    if agent.spec().initial_prompt.args("").is_none() {
        return Err(error(
            task_error::AGENT_UNSUPPORTED,
            format!("{} cannot be started on a Task", agent.label()),
        ));
    }
    // The last folder is looked at off the lock; the plan is made under it.
    let last_path = {
        let mut state = state.lock().or_abort();
        let task = state.task_mut(id)?;
        startable(task)?;
        task.last.as_ref().map(|last| last.path.clone())
    };
    let last_exists = last_path.as_deref().is_some_and(std::path::Path::is_dir);
    let (plan, title, description, files, worktree_root) = {
        let mut state = state.lock().or_abort();
        let task = state.task_mut(id)?;
        startable(task)?;
        let task = task.clone();
        let plan = state.plan(&task, place, last_exists)?;
        let tasks = state.tasks.as_mut().expect("checked by the request");
        if let Plan::Existing(workspace_id) = &plan {
            tasks.claims.insert(id, *workspace_id);
        }
        let files = tasks.file(id);
        state.task_mut(id)?.starting = true;
        state.task_changed(id);
        (
            plan,
            task.title,
            task.description.unwrap_or_default(),
            files,
            state.worktree_root.clone(),
        )
    };
    let mut reservation = Reservation {
        state,
        task: id,
        committed: false,
    };

    let mut made = Made::default();
    let result = prepare(
        state,
        lifecycle,
        id,
        &plan,
        &title,
        worktree_root,
        &mut made,
    )
    .map_err(|error| error.at("workspace"))
    .and_then(|(workspace_id, tab_id, pane_id)| {
        write_task_file(&files, &title, &description).map_err(|error| error.at("task_file"))?;
        type_agent(
            state,
            lifecycle,
            &mut reservation,
            id,
            agent,
            &files,
            (workspace_id, made.workspace.is_some()),
            pane_id,
        )
        .map_err(|error| error.at("agent"))?;
        Ok(TaskResponse::Started {
            workspace_id,
            tab_id,
            pane_id,
        })
    });
    if let Err(failure) = &result {
        undo_failed(state, made, lifecycle.is_stopping());
        tracing::info!(
            task = id.as_u64(),
            step = failure.step.as_deref().unwrap_or(""),
            code = %failure.code,
            "a Task did not start: {}",
            failure.message
        );
    }
    result
}

/// A Task a Start may take: neither archived, started nor already starting.
fn startable(task: &condr_core::Task) -> Result<(), TaskError> {
    if task.archived {
        return Err(error(
            task_error::TASK_ARCHIVED,
            "restore the Task before starting it",
        ));
    }
    if task.starting {
        return Err(error(
            task_error::TASK_STARTING,
            format!("Task {} is already starting", task.id.as_u64()),
        ));
    }
    if task.workspace.is_some() {
        return Err(error(
            task_error::TASK_STARTED,
            format!("Task {} is already started", task.id.as_u64()),
        ));
    }
    Ok(())
}

impl RuntimeState {
    /// Resolves `place`, or the default when there is none (ADR 0068): continue where
    /// the Task was worked, else its repository's Workspace when no Task holds it, else a
    /// new worktree of it; a Task without a repository takes the Device's first Workspace
    /// no Task holds.
    fn plan(
        &self,
        task: &condr_core::Task,
        place: Option<StartPlace>,
        last_exists: bool,
    ) -> Result<Plan, TaskError> {
        let tasks = self.tasks.as_ref().expect("checked by the request");
        let held = |workspace_id: WorkspaceId| {
            tasks
                .holder(workspace_id)
                .is_some_and(|holder| holder != task.id)
        };
        let repository_workspace = task
            .repository
            .as_deref()
            .and_then(|repository| self.session.workspace_by_root(repository));
        let place = match place {
            Some(place) => place,
            None => match &task.last {
                Some(last)
                    if last_exists
                        && self
                            .session
                            .workspace_by_root(&last.path)
                            .is_none_or(|workspace| !held(workspace.id())) =>
                {
                    StartPlace::Continue
                }
                // The folder is gone: a new worktree on the branch it had.
                Some(last) if !last_exists && repository_workspace.is_some() => {
                    StartPlace::NewWorktree {
                        branch: last.branch.clone(),
                    }
                }
                _ => match (&task.repository, repository_workspace) {
                    (Some(_), Some(workspace)) if !held(workspace.id()) => {
                        StartPlace::Workspace(workspace.id())
                    }
                    (Some(_), Some(_)) => StartPlace::NewWorktree { branch: None },
                    (Some(_), None) => return Err(where_required(task)),
                    (None, _) => self
                        .session
                        .workspaces()
                        .iter()
                        .find(|workspace| !held(workspace.id()))
                        .map(|workspace| StartPlace::Workspace(workspace.id()))
                        .ok_or_else(|| where_required(task))?,
                },
            },
        };
        let held_error = |workspace_id: WorkspaceId| {
            error(
                task_error::WORKSPACE_HAS_TASK,
                format!(
                    "Workspace {} belongs to another Task",
                    workspace_id.as_u64()
                ),
            )
        };
        match place {
            StartPlace::Continue => {
                let last = task.last.as_ref().ok_or_else(|| {
                    error(
                        task_error::NO_LAST_PLACE,
                        "the Task was never worked, so there is nothing to continue",
                    )
                })?;
                if let Some(workspace) = self.session.workspace_by_root(&last.path) {
                    if held(workspace.id()) {
                        return Err(held_error(workspace.id()));
                    }
                    return Ok(Plan::Existing(workspace.id()));
                }
                if !last_exists {
                    return Err(error(
                        task_error::FOLDER_MISSING,
                        format!("the folder no longer exists: {}", last.path.display()),
                    ));
                }
                // Condr's own record that it made the worktree, and its repository open.
                let managed = last
                    .managed
                    .then_some(last.repository.as_deref())
                    .flatten()
                    .and_then(|repository| self.session.workspace_by_root(repository))
                    .map(|parent| {
                        (
                            parent.id(),
                            parent.root_directory().to_path_buf(),
                            last.base_branch.clone(),
                        )
                    });
                Ok(Plan::Reopen {
                    path: last.path.clone(),
                    managed,
                })
            }
            StartPlace::Workspace(workspace_id) => {
                if self.session.workspace(workspace_id).is_none() {
                    return Err(error(
                        task_error::WORKSPACE_NOT_FOUND,
                        format!("no Workspace {}", workspace_id.as_u64()),
                    ));
                }
                if held(workspace_id) {
                    return Err(held_error(workspace_id));
                }
                Ok(Plan::Existing(workspace_id))
            }
            StartPlace::NewWorktree { branch } => {
                let repository = task.repository.as_deref().ok_or_else(|| {
                    error(
                        task_error::NO_REPOSITORY,
                        "a new worktree needs the Task's repository",
                    )
                })?;
                let parent = repository_workspace.ok_or_else(|| {
                    error(
                        task_error::REPOSITORY_NOT_OPEN,
                        format!("open {} first", repository.display()),
                    )
                })?;
                Ok(Plan::NewWorktree {
                    parent_workspace_id: parent.id(),
                    parent_root: parent.root_directory().to_path_buf(),
                    branch: branch
                        .map(|branch| branch.trim().to_owned())
                        .filter(|branch| !branch.is_empty())
                        .unwrap_or_else(|| default_task_branch(&task.title, repository)),
                })
            }
        }
    }
}

fn where_required(task: &condr_core::Task) -> TaskError {
    error(
        task_error::WHERE_REQUIRED,
        match &task.repository {
            Some(repository) => format!("open {} first", repository.display()),
            None => "open a Workspace on this Device first".into(),
        },
    )
}

/// Commits one layout change under the lock and starts the shells it made.
fn commit_layout(
    state: &Arc<Mutex<RuntimeState>>,
    lifecycle: &Arc<ServerLifecycle>,
    apply: impl FnOnce(&mut RuntimeState) -> Result<LayoutEffect, String>,
) -> Result<LayoutResult, String> {
    let mut guard = state.lock().or_abort();
    if lifecycle.is_stopping() {
        return Err("the Server is stopping".into());
    }
    let effect = apply(&mut guard)?;
    guard.publish_background(effect.event);
    drop(guard);
    start_monitors(state, lifecycle, effect.started_terminals);
    drop(effect.removed_terminals);
    Ok(effect.result)
}

/// The Workspace, its Tab and the Pane the Agent starts in.
fn prepare(
    state: &Arc<Mutex<RuntimeState>>,
    lifecycle: &Arc<ServerLifecycle>,
    id: TaskId,
    plan: &Plan,
    title: &str,
    worktree_root: Option<PathBuf>,
    made: &mut Made,
) -> Result<(WorkspaceId, TabId, PaneId), TaskError> {
    let failed = |message: String| error(task_error::WORKSPACE_FAILED, message);
    let created = |result: LayoutResult, made: &mut Made| match result {
        LayoutResult::WorkspaceCreated {
            workspace_id,
            tab_id,
            pane_id,
        } => {
            made.workspace = Some(workspace_id);
            Ok((workspace_id, tab_id, pane_id))
        }
        _ => Err(failed("the Workspace was not created".into())),
    };
    match plan {
        Plan::Existing(workspace_id) => {
            let result = commit_layout(state, lifecycle, |state| {
                apply_layout_command(
                    state,
                    LayoutCommand::CreateTab {
                        workspace_id: *workspace_id,
                        name: Some(title.to_owned()),
                        cwd_from: None,
                    },
                )
            })
            .map_err(failed)?;
            match result {
                LayoutResult::TabCreated { tab_id, pane_id } => {
                    made.tab = Some(tab_id);
                    Ok((*workspace_id, tab_id, pane_id))
                }
                _ => Err(failed("the Tab was not created".into())),
            }
        }
        Plan::Reopen { path, managed } => {
            // Managed again only when the folder is still a linked worktree of the
            // repository the record names (ADR 0068).
            let managed = managed.as_ref().and_then(|(parent_id, parent_root, base)| {
                let parent = discover_repository(parent_root).ok().flatten()?;
                let child = open_worktree(&parent, path).ok()?;
                let same = std::fs::canonicalize(child.root()).ok()?
                    == std::fs::canonicalize(path).ok()?;
                same.then(|| PreparedExternalLayout::CreateWorktree {
                    parent_workspace_id: *parent_id,
                    parent_root: parent_root.clone(),
                    parent,
                    child,
                    base_branch: base.clone(),
                    created_branch: false,
                })
            });
            let prepared = match managed {
                Some(prepared) => prepared,
                None => prepare_external_layout(ExternalLayoutPlan::CreateWorkspace {
                    root: path.clone(),
                    name: None,
                })
                .map_err(failed)?,
            };
            let result = commit_layout(state, lifecycle, |state| {
                let effect = apply_prepared_external_layout(state, prepared);
                claimed(state, id, effect)
            })
            .map_err(failed)?;
            created(result, made)
        }
        Plan::NewWorktree {
            parent_workspace_id,
            parent_root,
            branch,
        } => {
            let parent = discover_repository(parent_root)
                .map_err(|error| failed(error.to_string()))?
                .ok_or_else(|| failed(format!("{} is not a repository", parent_root.display())))?;
            match branch_is_checked_out(&parent, branch) {
                Ok(true) => {
                    return Err(error(
                        task_error::BRANCH_CHECKED_OUT,
                        format!("branch {branch} is already checked out"),
                    ));
                }
                Ok(false) => {}
                Err(git) => return Err(failed(git.to_string())),
            }
            let prepared = prepare_external_layout(ExternalLayoutPlan::CreateWorktree {
                parent_workspace_id: *parent_workspace_id,
                parent_root: parent_root.clone(),
                branch: branch.clone(),
                worktree_root,
            })
            .map_err(failed)?;
            if let PreparedExternalLayout::CreateWorktree {
                parent,
                child,
                created_branch,
                ..
            } = &prepared
            {
                made.worktree = Some((parent.clone(), child.clone()));
                if *created_branch && let Some(commit) = child.head() {
                    made.branch = Some((parent.clone(), branch.clone(), commit.to_owned()));
                }
            }
            let result = commit_layout(state, lifecycle, |state| {
                let effect = apply_prepared_external_layout(state, prepared);
                claimed(state, id, effect)
            })
            .map_err(failed)?;
            created(result, made)
        }
    }
}

/// Claims a Workspace a Start created as it is created, so no other Start takes it for
/// its own before this one links it.
fn claimed(
    state: &mut RuntimeState,
    id: TaskId,
    effect: Result<LayoutEffect, String>,
) -> Result<LayoutEffect, String> {
    if let Ok(LayoutEffect {
        result: LayoutResult::WorkspaceCreated { workspace_id, .. },
        ..
    }) = &effect
        && let Some(tasks) = state.tasks.as_mut()
    {
        tasks.claims.insert(id, *workspace_id);
    }
    effect
}

/// `tasks/<id>.md` in the data directory: the title, a blank line and the description,
/// outside the worktree so `git status` does not show it.
fn write_task_file(
    file: &std::path::Path,
    title: &str,
    description: &str,
) -> Result<(), TaskError> {
    use std::io::Write as _;
    // The person's alone, as the store is: owner-only directory and file.
    let written = file
        .parent()
        .map_or(Ok(()), crate::persistence::create_private_directory)
        .and_then(|()| {
            crate::persistence::private_file_options()
                .truncate(true)
                .open(file)?
                .write_all(format!("{title}\n\n{description}").as_bytes())
        });
    written.map_err(|io| {
        error(
            task_error::TASK_FILE_FAILED,
            format!("cannot write {}: {io}", file.display()),
        )
    })
}

/// The one short sentence the Agent starts on (ADR 0068).
pub(super) fn initial_prompt(file: &std::path::Path) -> String {
    format!("Read {} and do the task it describes.", file.display())
}

/// Refuses a prompt an npm `.cmd` shim would refuse, saying why (ADR 0068).
pub(super) fn check_shim(
    installation: &AgentInstallation,
    file: &std::path::Path,
) -> Result<(), TaskError> {
    let batch = installation
        .executable
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        });
    let path = file.to_string_lossy();
    if batch && path.contains(SHIM_REFUSES) {
        return Err(error(
            task_error::UNSUPPORTED_PATH,
            format!(
                "{} starts through an npm .cmd shim, which refuses the task file's path {path}: \
                 it holds one of & | < > ^ ( ) % ! \"; set CONDR_DATA_DIR to a directory \
                 without them",
                installation.executable.display()
            ),
        ));
    }
    Ok(())
}

fn installation(
    state: &Arc<Mutex<RuntimeState>>,
    agent: AgentKind,
) -> Result<AgentInstallation, TaskError> {
    #[cfg(test)]
    if let Some(agents) = state
        .lock()
        .or_abort()
        .tasks
        .as_ref()
        .and_then(|tasks| tasks.agents.clone())
    {
        return agents
            .into_iter()
            .find(|installation| installation.kind == agent)
            .ok_or_else(|| unavailable(agent));
    }
    #[cfg(not(test))]
    let _ = state;
    condr_core::agent_discovery::discover()
        .into_iter()
        .find(|installation| installation.kind == agent)
        .ok_or_else(|| unavailable(agent))
}

fn unavailable(agent: AgentKind) -> TaskError {
    error(
        task_error::AGENT_UNAVAILABLE,
        format!(
            "{} is not executable on the Server's PATH",
            agent.spec().executable
        ),
    )
}

/// Waits for the Pane's shell, types the Agent's command and, in the same step, links the
/// Task: the commit point.
#[allow(clippy::too_many_arguments)]
fn type_agent(
    state: &Arc<Mutex<RuntimeState>>,
    lifecycle: &ServerLifecycle,
    reservation: &mut Reservation<'_>,
    id: TaskId,
    agent: AgentKind,
    file: &std::path::Path,
    // The Workspace, and whether this Start made it rather than borrowing it.
    (workspace_id, made_workspace): (WorkspaceId, bool),
    pane_id: PaneId,
) -> Result<(), TaskError> {
    let installation = installation(state, agent)?;
    check_shim(&installation, file)?;
    let args = agent
        .spec()
        .initial_prompt
        .args(&initial_prompt(file))
        .expect("checked before reserving");
    let deadline = Instant::now() + SHELL_READY_TIMEOUT;
    let gone = || error(task_error::AGENT_START_FAILED, "the Pane closed");
    loop {
        if lifecycle.is_stopping() {
            return Err(error(task_error::SERVER_STOPPING, "the Server is stopping"));
        }
        let (instance, probe) = {
            let state = state.lock().or_abort();
            let instance = state.terminal_instances.get(&pane_id).copied();
            let probe = state
                .terminals
                .get(&pane_id)
                .map(TerminalRuntime::launch_probe);
            instance.zip(probe).ok_or_else(gone)?
        };
        // `command` looks at the shell itself: a prompt hook's child still running reads
        // as busy, which passes like a shell still starting.
        #[cfg(test)]
        let busy = state
            .lock()
            .or_abort()
            .tasks
            .as_ref()
            .is_some_and(|tasks| tasks.shells_busy);
        #[cfg(not(test))]
        let busy = false;
        let command = if busy {
            Err(io::Error::from(io::ErrorKind::WouldBlock))
        } else {
            probe.command(&installation, &args)
        };
        let command = match command {
            Ok(command) => command,
            Err(io) if io.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                thread::sleep(PROMPT_POLL_INTERVAL);
                continue;
            }
            Err(io) if io.kind() == io::ErrorKind::WouldBlock => {
                return Err(error(
                    task_error::SHELL_NOT_READY,
                    "the shell did not reach its prompt in 30 seconds",
                ));
            }
            Err(io) => return Err(error(task_error::AGENT_START_FAILED, io.to_string())),
        };
        let mut state = state.lock().or_abort();
        if !state.terminal_is_current(pane_id, instance)
            || state.closing_terminals.contains(&pane_id)
            || state.session.workspace(workspace_id).is_none()
        {
            return Err(gone());
        }
        match state.terminals[&pane_id].submit(&command) {
            Ok(()) => {}
            // The input queue is full for a moment.
            Err(io) if io.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                drop(state);
                thread::sleep(PROMPT_POLL_INTERVAL);
                continue;
            }
            Err(io) => return Err(error(task_error::AGENT_START_FAILED, io.to_string())),
        }
        // Committed: from here on nothing is undone.
        reservation.committed = true;
        let last = state.last_place(workspace_id);
        let tasks = state.tasks.as_mut().expect("checked by the request");
        tasks.claims.remove(&id);
        let task = state
            .task_mut(id)
            .expect("a starting Task is never deleted");
        task.starting = false;
        task.workspace = Some(workspace_id);
        task.made_workspace = made_workspace;
        task.last = last;
        state.task_changed(id);
        tracing::info!(
            task = id.as_u64(),
            workspace = workspace_id.as_u64(),
            agent = agent.id(),
            "a Task started"
        );
        return Ok(());
    }
}

/// Undoes what a failed Start made before its commit point: the Workspace or the Tab, then
/// the worktree, then a branch still where it was made. A stopping Server keeps a
/// Workspace it made, which returns without a Task (ADR 0068), and only takes back a
/// worktree and branch no Workspace was made for.
pub(super) fn undo_failed(state: &Arc<Mutex<RuntimeState>>, mut made: Made, stopping: bool) {
    if stopping {
        if made.workspace.is_some() {
            return;
        }
        made.tab = None;
    }
    let close = {
        let state = state.lock().or_abort();
        match (made.workspace, made.tab) {
            (Some(workspace_id), _) => Some(LayoutCommand::CloseWorkspace { workspace_id }),
            // Closing a Workspace's last Tab would close the Workspace, which this Start
            // did not make.
            (None, Some(tab_id))
                if state.session.workspaces().iter().any(|workspace| {
                    workspace.tab(tab_id).is_some() && workspace.tabs().len() > 1
                }) =>
            {
                Some(LayoutCommand::CloseTab { tab_id })
            }
            _ => None,
        }
    };
    if let Some(command) = close {
        let removed = {
            let mut state = state.lock().or_abort();
            match apply_layout_command(&mut state, command) {
                Ok(effect) => {
                    state.publish_background(effect.event);
                    effect.removed_terminals
                }
                Err(reason) => {
                    tracing::warn!("a failed Task Start left its Workspace or Tab: {reason}");
                    Vec::new()
                }
            }
        };
        // The shells end before their worktree goes.
        drop(removed);
    }
    if let Some((parent, child)) = &made.worktree
        && let Err(error) = remove_worktree(parent, child)
    {
        tracing::warn!(
            "a failed Task Start left its worktree {}: {error}",
            child.root().display()
        );
    }
    if let Some((repository, branch, commit)) = &made.branch
        && let Err(error) = delete_branch_at(repository, branch, commit)
    {
        tracing::warn!("a failed Task Start left its branch {branch}: {error}");
    }
}
