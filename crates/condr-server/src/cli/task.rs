use super::workspace::{tab_info, workspace_info};
use super::*;
use condr_core::{StartPlace, Task, TaskCommand as Request, TaskId, TaskResponse};
use std::io::Read as _;

#[derive(Subcommand)]
pub(crate) enum TaskCommand {
    /// List the Tasks in To do and started ones, in order and without descriptions
    List {
        /// List the archived Tasks instead
        #[arg(long)]
        archived: bool,
    },
    /// Show one Task with its description
    Get {
        /// Task id; inside a Pane, the Task holding its Workspace
        task: Option<TaskId>,
    },
    /// Write a Task; it heads To do
    Create {
        /// One line
        #[arg(long, value_name = "TEXT")]
        title: String,
        #[command(flatten)]
        description: Description,
        /// The repository Workspace the Task works in
        #[arg(long, value_name = "WORKSPACE_ID")]
        repository: Option<u64>,
    },
    /// Change a Task's title, description or repository
    Edit {
        /// Task id; inside a Pane, the Task holding its Workspace
        task: Option<TaskId>,
        #[arg(long, value_name = "TEXT")]
        title: Option<String>,
        #[command(flatten)]
        description: Description,
        /// The repository Workspace the Task works in
        #[arg(long, value_name = "WORKSPACE_ID")]
        repository: Option<u64>,
    },
    /// Start an Agent on a To do Task; without a place, where the GUI would start it
    Start {
        /// Task id; inside a Pane, the Task holding its Workspace
        task: Option<TaskId>,
        /// Agent kind (see `condr agent available`)
        #[arg(long)]
        kind: String,
        /// A new Tab in this Workspace
        #[arg(long, value_name = "ID", conflicts_with_all = ["worktree", "resume"])]
        workspace: Option<u64>,
        /// A new worktree of the Task's repository, on BRANCH or a branch named after
        /// the title
        #[arg(long, value_name = "BRANCH", num_args = 0..=1, conflicts_with = "resume")]
        worktree: Option<Option<String>>,
        /// Where the Task was last worked
        #[arg(long = "continue")]
        resume: bool,
        /// Ask every connected GUI to show the Task's Workspace
        #[arg(long)]
        focus: bool,
    },
    /// Archive a Task; its Workspace stays open unless told
    Archive {
        /// Task id; inside a Pane, the Task holding its Workspace
        task: Option<TaskId>,
        /// Close the started Task's Workspace and stop its terminals
        #[arg(long)]
        close_workspace: bool,
        /// Also remove the Workspace's Managed Worktree; its branch stays
        #[arg(long, requires = "close_workspace")]
        remove_worktree: bool,
    },
    /// Put an archived Task back in To do
    Restore {
        /// Task id; inside a Pane, the Task holding its Workspace
        task: Option<TaskId>,
    },
    /// Delete a To do or archived Task for good
    Delete {
        /// Task id; inside a Pane, the Task holding its Workspace
        task: Option<TaskId>,
    },
}

#[derive(clap::Args)]
#[group(multiple = false)]
pub(crate) struct Description {
    /// Plain text sent to the Agent with the title
    #[arg(long, value_name = "TEXT")]
    description: Option<String>,
    /// Read the description from a file, or `-` for stdin
    #[arg(long, value_name = "PATH")]
    description_file: Option<PathBuf>,
}

impl Description {
    fn read(self) -> Result<Option<String>, CliError> {
        Ok(match self.description_file {
            Some(path) if path.as_os_str() == "-" => {
                let mut text = String::new();
                io::stdin().read_to_string(&mut text)?;
                Some(text)
            }
            Some(path) => Some(std::fs::read_to_string(path)?),
            None => self.description,
        })
    }
}

pub(crate) fn run_task(device: Option<&str>, command: TaskCommand) -> i32 {
    run(device, |client| task(client, device, command))
}

/// One Task request; an older Server, or one built without Tasks, answers the frozen
/// `UNSUPPORTED_REQUEST` (ADR 0068).
fn request(client: &mut ClientConnection, command: Request) -> Result<TaskResponse, CliError> {
    match client.task(command) {
        Ok(result) => result.map_err(|error| CliError {
            code: error.code,
            message: error.message,
            step: error.step,
        }),
        Err(error) if error.to_string() == condr_core::protocol::UNSUPPORTED_REQUEST => {
            Err(CliError::new(
                "feature_unavailable",
                "this Server keeps no Tasks: it predates them or was built without them",
            ))
        }
        Err(error) => Err(error.into()),
    }
}

fn unexpected() -> CliError {
    CliError::new("unexpected_response", "the Server answered another request")
}

fn one(response: TaskResponse) -> Result<Value, CliError> {
    match response {
        TaskResponse::Task(task) => Ok(task_info(&task)),
        _ => Err(unexpected()),
    }
}

/// A Task as the CLI prints it; `description` only when the Server sent it, as `get`
/// does and `list` does not.
fn task_info(task: &Task) -> Value {
    let mut info = json!({
        "task": task.id.to_string(),
        "title": task.title,
        "status": task.status().as_str(),
        "repository": task.repository,
        "workspace": task.workspace.map(WorkspaceId::as_u64),
        "last": task.last.as_ref().map(|last| json!({
            "branch": last.branch,
            "base": last.base_branch,
            "path": last.path,
            "managed": last.managed,
        })),
    });
    if let Some(description) = &task.description {
        info["description"] = description.as_str().into();
    }
    info
}

/// `id`, or the Task holding the calling Pane's Workspace.
fn task_id(
    client: &mut ClientConnection,
    device: Option<&str>,
    id: Option<TaskId>,
) -> Result<TaskId, CliError> {
    if let Some(id) = id {
        return Ok(id);
    }
    let pane = PaneId::from_u64(super::pane::caller_pane(device)?);
    let workspace = client
        .session()?
        .workspace_for_pane(pane)
        .map(Workspace::id);
    let TaskResponse::Tasks(tasks) = request(client, Request::List { archived: false })? else {
        return Err(unexpected());
    };
    tasks
        .into_iter()
        .find(|task| workspace.is_some() && task.workspace == workspace)
        .map(|task| task.id)
        .ok_or_else(|| {
            CliError::new(
                "no_task",
                "no Task holds the calling Pane's Workspace; pass a task id",
            )
        })
}

/// The root directory of Workspace `id`, which names a Task's repository.
fn repository(client: &ClientConnection, id: u64) -> Result<PathBuf, CliError> {
    Ok(find_workspace(&client.session()?, id)?
        .root_directory()
        .to_path_buf())
}

fn task(
    client: &mut ClientConnection,
    device: Option<&str>,
    command: TaskCommand,
) -> Result<Value, CliError> {
    match command {
        TaskCommand::List { archived } => match request(client, Request::List { archived })? {
            TaskResponse::Tasks(tasks) => Ok(json!({
                "tasks": tasks.iter().map(task_info).collect::<Vec<_>>()
            })),
            _ => Err(unexpected()),
        },
        TaskCommand::Get { task } => {
            let task = task_id(client, device, task)?;
            one(request(client, Request::Get { task })?)
        }
        TaskCommand::Create {
            title,
            description,
            repository: workspace,
        } => {
            let command = Request::Create {
                title,
                description: description.read()?.unwrap_or_default(),
                repository: workspace.map(|id| repository(client, id)).transpose()?,
            };
            one(request(client, command)?)
        }
        TaskCommand::Edit {
            task,
            title,
            description,
            repository: workspace,
        } => {
            let command = Request::Edit {
                task: task_id(client, device, task)?,
                title,
                description: description.read()?,
                repository: workspace
                    .map(|id| repository(client, id).map(Some))
                    .transpose()?,
            };
            one(request(client, command)?)
        }
        TaskCommand::Start {
            task,
            kind,
            workspace,
            worktree,
            resume,
            focus,
        } => {
            let agent = AgentKind::parse_label(&kind).ok_or_else(|| {
                CliError::new("unknown_agent_kind", format!("unknown agent kind {kind}"))
            })?;
            let place = match (workspace, worktree, resume) {
                (Some(id), _, _) => Some(StartPlace::Workspace(WorkspaceId::from_u64(id))),
                (_, Some(branch), _) => Some(StartPlace::NewWorktree { branch }),
                (_, _, true) => Some(StartPlace::Continue),
                _ => None,
            };
            let task = task_id(client, device, task)?;
            let TaskResponse::Started {
                workspace_id,
                tab_id,
                pane_id,
            } = request(client, Request::Start { task, place, agent })?
            else {
                return Err(unexpected());
            };
            if focus {
                apply(client, LayoutCommand::ActivateWorkspace { workspace_id })?;
            }
            // A Start's layout changes reach only the Clients that subscribed, which this
            // one did not; a fresh overview has them.
            let fresh = connect(device)?;
            let session = fresh.session()?;
            let workspace = find_workspace(&session, workspace_id.as_u64())?;
            let tab = workspace
                .tab(tab_id)
                .ok_or_else(|| CliError::new("tab_not_found", "the started Tab vanished"))?;
            Ok(json!({
                "workspace": workspace_info(workspace),
                "tab": tab_info(workspace, tab),
                "pane": pane_info(&fresh, workspace, tab, pane_id),
            }))
        }
        TaskCommand::Archive {
            task,
            close_workspace,
            remove_worktree,
        } => {
            let task = task_id(client, device, task)?;
            one(request(
                client,
                Request::Archive {
                    task,
                    close_workspace,
                    remove_worktree,
                },
            )?)
        }
        TaskCommand::Restore { task } => {
            let task = task_id(client, device, task)?;
            one(request(client, Request::Restore { task })?)
        }
        TaskCommand::Delete { task } => {
            let task = task_id(client, device, task)?;
            request(client, Request::Delete { task })?;
            Ok(json!({ "ok": true }))
        }
    }
}
