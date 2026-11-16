//! Tasks on the wire (ADR 0068).

use super::session::command_agent_kind;
use super::*;
use crate::{
    LastPlace, MAX_TASK_DESCRIPTION_BYTES, PaneId, StartPlace, TabId, Task, TaskCommand, TaskError,
    TaskId, TaskResponse, WorkspaceId,
};

pub(super) fn encode_task(task: &Task) -> Result<super::Task, String> {
    Ok(super::Task {
        id: task.id.as_u64(),
        title: task.title.clone(),
        description: task.description.clone(),
        repository: optional_path_text(task.repository.as_ref())?,
        order: task.order,
        archived: task.archived,
        workspace: task.workspace.map(WorkspaceId::as_u64),
        made_workspace: task.made_workspace,
        starting: task.starting,
        last: task
            .last
            .as_ref()
            .map(|last| {
                Ok::<_, String>(TaskPlace {
                    path: path_text(&last.path)?,
                    branch: last.branch.clone(),
                    base_branch: last.base_branch.clone(),
                    managed: last.managed,
                    repository: optional_path_text(last.repository.as_ref())?,
                })
            })
            .transpose()?,
    })
}

pub(super) fn decode_task(task: super::Task) -> WireResult<Task> {
    if task
        .description
        .as_ref()
        .is_some_and(|description| description.len() > MAX_TASK_DESCRIPTION_BYTES)
    {
        return Err(malformed("a Task description exceeds 64 KiB"));
    }
    Ok(Task {
        id: TaskId::from_u64(task.id),
        title: task.title,
        description: task.description,
        repository: task.repository.map(PathBuf::from),
        order: task.order,
        archived: task.archived,
        workspace: task.workspace.map(WorkspaceId::from_u64),
        made_workspace: task.made_workspace,
        starting: task.starting,
        last: task.last.map(|last| LastPlace {
            path: PathBuf::from(last.path),
            branch: last.branch,
            base_branch: last.base_branch,
            managed: last.managed,
            repository: last.repository.map(PathBuf::from),
        }),
    })
}

pub(super) fn encode_tasks(tasks: &[Task]) -> Result<Tasks, String> {
    Ok(Tasks {
        tasks: tasks.iter().map(encode_task).collect::<Result<_, _>>()?,
    })
}

pub(super) fn decode_tasks(tasks: Tasks) -> WireResult<Vec<Task>> {
    tasks.tasks.into_iter().map(decode_task).collect()
}

impl TryFrom<&TaskCommand> for super::TaskCommand {
    type Error = String;

    fn try_from(command: &TaskCommand) -> Result<Self, String> {
        use task_command::Command;
        Ok(Self {
            command: Some(match command {
                TaskCommand::Create {
                    title,
                    description,
                    repository,
                } => Command::Create(TaskCreate {
                    title: title.clone(),
                    description: description.clone(),
                    repository: optional_path_text(repository.as_ref())?,
                }),
                TaskCommand::Edit {
                    task,
                    title,
                    description,
                    repository,
                } => Command::Edit(TaskEdit {
                    task: task.as_u64(),
                    title: title.clone(),
                    description: description.clone(),
                    repository: repository
                        .as_ref()
                        .map(|path| {
                            Ok::<_, String>(TaskRepository {
                                path: optional_path_text(path.as_ref())?,
                            })
                        })
                        .transpose()?,
                }),
                TaskCommand::Delete { task } => Command::Delete(task.as_u64()),
                TaskCommand::Archive {
                    task,
                    close_workspace,
                    remove_worktree,
                } => Command::Archive(TaskArchive {
                    task: task.as_u64(),
                    close_workspace: *close_workspace,
                    remove_worktree: *remove_worktree,
                }),
                TaskCommand::Restore { task } => Command::Restore(task.as_u64()),
                TaskCommand::Reorder { task, index } => Command::Reorder(TaskReorder {
                    task: task.as_u64(),
                    index: *index,
                }),
                TaskCommand::Get { task } => Command::Get(task.as_u64()),
                TaskCommand::List { archived } => Command::List(TaskList {
                    archived: *archived,
                }),
                TaskCommand::Start { task, place, agent } => Command::Start(TaskStart {
                    task: task.as_u64(),
                    place: place.as_ref().map(|place| {
                        use start_place::Place;
                        super::StartPlace {
                            place: Some(match place {
                                StartPlace::Continue => Place::Continue(Empty {}),
                                StartPlace::Workspace(workspace) => {
                                    Place::Workspace(workspace.as_u64())
                                }
                                StartPlace::NewWorktree { branch } => {
                                    Place::NewWorktree(NewWorktree {
                                        branch: branch.clone(),
                                    })
                                }
                            }),
                        }
                    }),
                    agent: super::AgentKind::from(*agent) as i32,
                }),
            }),
        })
    }
}

impl TryFrom<super::TaskCommand> for TaskCommand {
    type Error = WireError;

    fn try_from(command: super::TaskCommand) -> WireResult<Self> {
        use task_command::Command;
        let task = TaskId::from_u64;
        Ok(match member(command.command)? {
            Command::Create(create) => Self::Create {
                title: create.title,
                description: create.description,
                repository: create.repository.map(PathBuf::from),
            },
            Command::Edit(edit) => Self::Edit {
                task: task(edit.task),
                title: edit.title,
                description: edit.description,
                repository: edit
                    .repository
                    .map(|repository| repository.path.map(PathBuf::from)),
            },
            Command::Delete(id) => Self::Delete { task: task(id) },
            Command::Archive(archive) => Self::Archive {
                task: task(archive.task),
                close_workspace: archive.close_workspace,
                remove_worktree: archive.remove_worktree,
            },
            Command::Restore(id) => Self::Restore { task: task(id) },
            Command::Reorder(reorder) => Self::Reorder {
                task: task(reorder.task),
                index: reorder.index,
            },
            Command::Get(id) => Self::Get { task: task(id) },
            Command::List(list) => Self::List {
                archived: list.archived,
            },
            Command::Start(start) => Self::Start {
                task: task(start.task),
                place: start
                    .place
                    .map(|place| {
                        use start_place::Place;
                        Ok::<_, WireError>(match member(place.place)? {
                            Place::Continue(_) => StartPlace::Continue,
                            Place::Workspace(id) => {
                                StartPlace::Workspace(WorkspaceId::from_u64(id))
                            }
                            Place::NewWorktree(worktree) => StartPlace::NewWorktree {
                                branch: worktree.branch,
                            },
                        })
                    })
                    .transpose()?,
                agent: command_agent_kind(start.agent)?,
            },
        })
    }
}

pub(super) fn encode_task_reply(
    request_id: u64,
    result: &Result<TaskResponse, TaskError>,
) -> Result<TaskReply, String> {
    use task_reply::Result as Wire;
    use task_response::Response;
    Ok(TaskReply {
        request_id,
        result: Some(match result {
            Ok(response) => Wire::Ok(super::TaskResponse {
                response: Some(match response {
                    TaskResponse::Task(task) => Response::Task(encode_task(task)?),
                    TaskResponse::Tasks(tasks) => Response::Tasks(encode_tasks(tasks)?),
                    TaskResponse::Started {
                        workspace_id,
                        tab_id,
                        pane_id,
                    } => Response::Started(TaskStarted {
                        workspace_id: workspace_id.as_u64(),
                        tab_id: tab_id.as_u64(),
                        pane_id: pane_id.as_u64(),
                    }),
                    TaskResponse::Done => Response::Done(Empty {}),
                }),
            }),
            Err(error) => Wire::Err(super::TaskError {
                code: error.code.clone(),
                message: error.message.clone(),
                step: error.step.clone(),
            }),
        }),
    })
}

pub(super) fn decode_task_reply(reply: TaskReply) -> WireResult<Result<TaskResponse, TaskError>> {
    use task_reply::Result as Wire;
    use task_response::Response;
    Ok(match member(reply.result)? {
        Wire::Ok(response) => Ok(match member(response.response)? {
            Response::Task(task) => TaskResponse::Task(decode_task(task)?),
            Response::Tasks(tasks) => TaskResponse::Tasks(decode_tasks(tasks)?),
            Response::Started(started) => TaskResponse::Started {
                workspace_id: WorkspaceId::from_u64(started.workspace_id),
                tab_id: TabId::from_u64(started.tab_id),
                pane_id: PaneId::from_u64(started.pane_id),
            },
            Response::Done(_) => TaskResponse::Done,
        }),
        Wire::Err(error) => Err(TaskError {
            code: error.code,
            message: error.message,
            step: error.step,
        }),
    })
}
