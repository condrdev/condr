//! `StartTask` against a real Server, shells and Git (ADR 0068). The Agent is a script
//! that writes the arguments it was started with.

use super::*;
use condr_core::agent_discovery::AgentInstallation;

struct Fixture {
    root: PathBuf,
    handle: ServerHandle,
    endpoint: Endpoint,
    thread: Option<thread::JoinHandle<io::Result<()>>>,
}

impl Fixture {
    fn new(name: &str) -> Self {
        Self::in_data(name, "data")
    }

    fn in_data(name: &str, data: &str) -> Self {
        let root = temp(name);
        let (handle, endpoint, thread) = tasks_server(&root.join(data));
        Self {
            root,
            handle,
            endpoint,
            thread: Some(thread),
        }
    }

    fn connect(&self) -> ClientConnection {
        ClientConnection::connect(&self.endpoint, hinted()).unwrap()
    }

    /// Starts `kind` as a script that writes its arguments to `args`, one per line.
    fn agent(&self, kind: AgentKind, executable: &str) -> PathBuf {
        let bin = self.root.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let args = self.root.join(format!("{executable}.args"));
        let script = bin.join(executable);
        std::fs::write(
            &script,
            format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n", args.display()),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        self.agents(vec![AgentInstallation {
            kind,
            executable: script,
        }]);
        args
    }

    fn agents(&self, agents: Vec<AgentInstallation>) {
        self.handle
            .state
            .lock()
            .unwrap()
            .tasks
            .as_mut()
            .unwrap()
            .agents = Some(agents);
    }

    fn session(&self) -> Session {
        Session::restore(self.handle.snapshot()).unwrap()
    }

    fn file(&self, task: TaskId) -> PathBuf {
        self.handle
            .state
            .lock()
            .unwrap()
            .tasks
            .as_ref()
            .unwrap()
            .file(task)
    }

    /// A repository with one commit, and a Workspace open on it.
    fn repository(&self, connection: &mut ClientConnection) -> (PathBuf, WorkspaceId) {
        let repository = self.root.join("api");
        std::fs::create_dir_all(&repository).unwrap();
        run_git(&repository, &["init", "-q", "-b", "main"]);
        run_git(&repository, &["config", "user.name", "Condr Tests"]);
        run_git(
            &repository,
            &["config", "user.email", "condr@example.invalid"],
        );
        run_git(&repository, &["config", "commit.gpgsign", "false"]);
        std::fs::write(repository.join("README.md"), "api\n").unwrap();
        run_git(&repository, &["add", "README.md"]);
        run_git(&repository, &["commit", "-q", "-m", "initial"]);
        let workspace = self.workspace(connection, &repository);
        (repository, workspace)
    }

    fn workspace(&self, connection: &mut ClientConnection, root: &std::path::Path) -> WorkspaceId {
        match connection
            .layout(LayoutCommand::CreateWorkspace {
                root_directory: root.to_path_buf(),
                name: None,
            })
            .unwrap()
        {
            Ok(LayoutResult::WorkspaceCreated { workspace_id, .. }) => workspace_id,
            other => panic!("{other:?}"),
        }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.handle.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn new_task(
    connection: &mut ClientConnection,
    title: &str,
    repository: Option<&std::path::Path>,
) -> TaskId {
    task_of(
        connection,
        TaskCommand::Create {
            title: title.into(),
            description: "Make it work.\nThen test it.".into(),
            repository: repository.map(std::path::Path::to_path_buf),
        },
    )
    .id
}

fn start_task(
    connection: &mut ClientConnection,
    task: TaskId,
    place: Option<StartPlace>,
) -> Result<(WorkspaceId, condr_core::TabId, PaneId), TaskError> {
    connection
        .task(TaskCommand::Start {
            task,
            place,
            agent: AgentKind::Claude,
        })
        .unwrap()
        .map(|response| match response {
            TaskResponse::Started {
                workspace_id,
                tab_id,
                pane_id,
            } => (workspace_id, tab_id, pane_id),
            other => panic!("{other:?}"),
        })
}

/// What the Agent was started with, once its shell ran it.
fn agent_args(args: &std::path::Path) -> Vec<String> {
    for _ in 0..600 {
        if let Ok(text) = std::fs::read_to_string(args)
            && text.ends_with('\n')
        {
            return text.lines().map(str::to_owned).collect();
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("the Agent never started");
}

fn get(connection: &mut ClientConnection, task: TaskId) -> Task {
    task_of(connection, TaskCommand::Get { task })
}

fn branch_exists(repository: &std::path::Path, branch: &str) -> bool {
    std::process::Command::new("git")
        .arg("-C")
        .arg(repository)
        .args([
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/heads/{branch}"),
        ])
        .output()
        .unwrap()
        .status
        .success()
}

/// ADR 0068: a Task without a repository starts in the Device's first Workspace by
/// default, in a new Tab named after it; its file is written once, the Agent is typed
/// with one sentence naming it, and archiving removes the file.
#[test]
fn a_task_starts_in_a_new_tab_with_its_file_and_one_sentence() {
    let fixture = Fixture::new("existing");
    let args = fixture.agent(AgentKind::Claude, "claude");
    let mut connection = fixture.connect();
    let folder = fixture.root.join("notes");
    std::fs::create_dir_all(&folder).unwrap();
    let workspace = fixture.workspace(&mut connection, &folder);
    let task = new_task(&mut connection, "Write the notes", None);

    let (started_in, tab, pane) = start_task(&mut connection, task, None).unwrap();
    assert_eq!(started_in, workspace);
    let file = fixture.file(task);
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "Write the notes\n\nMake it work.\nThen test it."
    );
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode =
            |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&file), 0o600, "the task file is the person's alone");
        assert_eq!(mode(file.parent().unwrap()), 0o700);
    }
    assert_eq!(
        agent_args(&args),
        [format!(
            "Read {} and do the task it describes.",
            file.display()
        )]
    );
    let session = fixture.session();
    let workspace_tabs = session.workspace(workspace).unwrap().tabs();
    assert_eq!(workspace_tabs.len(), 2);
    assert_eq!(session.tab(tab).unwrap().name(), "Write the notes");
    assert!(
        session
            .tab(tab)
            .unwrap()
            .panes()
            .iter()
            .any(|p| p.id() == pane)
    );
    let started = get(&mut connection, task);
    assert_eq!(started.workspace, Some(workspace));
    assert!(!started.starting);
    assert_eq!(started.last.unwrap().path, folder);

    // Started: it holds the Workspace, which no other Task may take.
    let other = new_task(&mut connection, "Another", None);
    assert_eq!(
        start_task(
            &mut connection,
            other,
            Some(StartPlace::Workspace(workspace))
        )
        .unwrap_err()
        .code,
        task_error::WORKSPACE_HAS_TASK
    );
    assert_eq!(
        start_task(&mut connection, other, None).unwrap_err().code,
        task_error::WHERE_REQUIRED,
        "no Workspace is free"
    );
    assert_eq!(
        start_task(&mut connection, task, None).unwrap_err().code,
        task_error::TASK_STARTED
    );

    let archived = task_of(
        &mut connection,
        TaskCommand::Archive {
            task,
            close_workspace: false,
            remove_worktree: false,
        },
    );
    assert!(archived.archived && archived.workspace.is_none());
    assert!(!file.exists(), "archiving removes the task file");
    assert!(
        fixture.session().workspace(workspace).is_some(),
        "left open"
    );
    assert_eq!(
        connection.task(TaskCommand::Delete { task }).unwrap(),
        Ok(TaskResponse::Done)
    );
}

/// ADR 0068: before the commit point a failed Start closes what it opened, removes the
/// worktree it created and deletes the branch it made, and the Task stays in To do.
#[test]
fn a_start_that_fails_before_typing_undoes_what_it_made() {
    let fixture = Fixture::new("rollback");
    fixture.agents(Vec::new());
    let mut connection = fixture.connect();
    let (repository, repository_workspace) = fixture.repository(&mut connection);
    let task = new_task(&mut connection, "Fix the login bug", Some(&repository));

    let failed = start_task(
        &mut connection,
        task,
        Some(StartPlace::NewWorktree { branch: None }),
    )
    .unwrap_err();
    assert_eq!(failed.code, task_error::AGENT_UNAVAILABLE);
    assert_eq!(failed.step.as_deref(), Some("agent"));
    let session = fixture.session();
    assert_eq!(session.workspaces().len(), 1, "the new Workspace closed");
    assert!(!branch_exists(&repository, "worktree/fix-the-login-bug"));
    assert!(
        !condr_core::worktree_destination(&repository, "worktree/fix-the-login-bug", None).exists(),
        "the worktree is removed"
    );
    let todo = get(&mut connection, task);
    assert!(!todo.starting && todo.workspace.is_none());

    // A branch that existed before the Start stays.
    run_git(&repository, &["branch", "worktree/kept"]);
    start_task(
        &mut connection,
        task,
        Some(StartPlace::NewWorktree {
            branch: Some("worktree/kept".into()),
        }),
    )
    .unwrap_err();
    assert!(branch_exists(&repository, "worktree/kept"));

    // In an existing Workspace, the Tab it opened closes.
    let failed = start_task(
        &mut connection,
        task,
        Some(StartPlace::Workspace(repository_workspace)),
    )
    .unwrap_err();
    assert_eq!(failed.step.as_deref(), Some("agent"));
    assert_eq!(
        fixture
            .session()
            .workspace(repository_workspace)
            .unwrap()
            .tabs()
            .len(),
        1
    );
    assert_eq!(
        start_task(
            &mut connection,
            task,
            Some(StartPlace::NewWorktree {
                branch: Some("main".into())
            })
        )
        .unwrap_err()
        .code,
        task_error::BRANCH_CHECKED_OUT
    );
    assert!(!get(&mut connection, task).starting);
}

/// ADR 0068: a new worktree's Start commits once the command is typed; an Agent that
/// exits at once undoes nothing. Continue reopens the folder as a Managed Worktree only
/// while the repository's Workspace is open, uses a Workspace open on it, and refuses a
/// folder that is gone.
#[test]
fn a_new_worktree_start_commits_and_continue_follows_the_record() {
    let fixture = Fixture::new("worktree");
    let args = fixture.agent(AgentKind::Claude, "claude");
    let mut connection = fixture.connect();
    let (repository, repository_workspace) = fixture.repository(&mut connection);
    let task = new_task(&mut connection, "修复输入法", Some(&repository));
    let branch = condr_core::default_task_branch("修复输入法", &repository);
    assert!(branch.starts_with("worktree/api-"), "{branch}");

    let (workspace, _, _) = start_task(
        &mut connection,
        task,
        Some(StartPlace::NewWorktree { branch: None }),
    )
    .unwrap();
    agent_args(&args);
    thread::sleep(Duration::from_millis(300));
    let session = fixture.session();
    let created = session.workspace(workspace).expect("nothing is undone");
    let association = created.worktree().unwrap();
    assert!(association.is_managed());
    assert_eq!(association.parent_workspace_id(), repository_workspace);
    let folder = created.root_directory().to_path_buf();
    assert!(branch_exists(&repository, &branch));
    let started = get(&mut connection, task);
    assert_eq!(started.workspace, Some(workspace));
    let last = started.last.unwrap();
    assert!(last.managed);
    assert_eq!(last.branch.as_deref(), Some(branch.as_str()));
    assert_eq!(last.repository.as_deref(), Some(repository.as_path()));

    // Closed, it is back in To do; Continue is the default and re-associates it.
    let close = |connection: &mut ClientConnection, workspace_id| {
        connection
            .layout(LayoutCommand::CloseWorkspace { workspace_id })
            .unwrap()
            .unwrap();
    };
    close(&mut connection, workspace);
    assert_eq!(get(&mut connection, task).workspace, None);
    let (reopened, _, _) = start_task(&mut connection, task, None).unwrap();
    let association = fixture
        .session()
        .workspace(reopened)
        .unwrap()
        .worktree()
        .cloned()
        .expect("a Managed Worktree again");
    assert!(association.is_managed());

    // Without the repository's Workspace, it opens as a plain Workspace.
    close(&mut connection, reopened);
    close(&mut connection, repository_workspace);
    let (plain, _, _) = start_task(&mut connection, task, Some(StartPlace::Continue)).unwrap();
    assert!(
        fixture
            .session()
            .workspace(plain)
            .unwrap()
            .worktree()
            .is_none()
    );

    // A Workspace already open on the folder is used, in a new Tab.
    close(&mut connection, plain);
    let open = fixture.workspace(&mut connection, &folder);
    let (used, _, _) = start_task(&mut connection, task, Some(StartPlace::Continue)).unwrap();
    assert_eq!(used, open);
    assert_eq!(fixture.session().workspace(open).unwrap().tabs().len(), 2);

    // A folder that is gone cannot be continued.
    close(&mut connection, open);
    std::fs::remove_dir_all(&folder).unwrap();
    assert_eq!(
        start_task(&mut connection, task, Some(StartPlace::Continue))
            .unwrap_err()
            .code,
        task_error::FOLDER_MISSING
    );

    // Back in To do, its file stays until it is deleted.
    let file = fixture.file(task);
    assert!(file.exists());
    assert_eq!(
        connection.task(TaskCommand::Delete { task }).unwrap(),
        Ok(TaskResponse::Done)
    );
    assert!(!file.exists(), "deleting removes the task file");
}

/// ADR 0068: a second Start of a Task under way is refused with `task_starting`, and a
/// Start whose requester leaves still finishes on the Server.
#[test]
fn a_second_start_is_refused_and_a_start_outlives_its_requester() {
    let fixture = Fixture::new("concurrent");
    let args = fixture.agent(AgentKind::Claude, "claude");
    let mut connection = fixture.connect();
    let folder = fixture.root.join("notes");
    std::fs::create_dir_all(&folder).unwrap();
    fixture.workspace(&mut connection, &folder);
    let task = new_task(&mut connection, "Twice", None);
    let server_id = connection.overview().server_id;

    let mut stream = connection.into_stream();
    for request_id in [100, 101] {
        condr_core::protocol::write_message(
            &mut stream,
            &ClientMessage::Task {
                server_id,
                request_id,
                command: TaskCommand::Start {
                    task,
                    place: None,
                    agent: AgentKind::Claude,
                },
            },
        )
        .unwrap();
    }
    let mut answers = Vec::new();
    while answers.len() < 2 {
        if let ServerMessage::Task { result, .. } = read_server(&mut stream) {
            answers.push(result);
        }
    }
    assert_eq!(answers.iter().filter(|answer| answer.is_ok()).count(), 1);
    assert!(answers.iter().any(|answer| matches!(
        answer,
        Err(error) if error.code == task_error::TASK_STARTING
    )));
    agent_args(&args);
    drop(stream);

    // The requester leaves right after asking; the Start goes on.
    let mut connection = fixture.connect();
    let second_folder = fixture.root.join("second");
    std::fs::create_dir_all(&second_folder).unwrap();
    fixture.workspace(&mut connection, &second_folder);
    let later = new_task(&mut connection, "Later", None);
    let mut leaving = fixture.connect().into_stream();
    condr_core::protocol::write_message(
        &mut leaving,
        &ClientMessage::Task {
            server_id,
            request_id: 1,
            command: TaskCommand::Start {
                task: later,
                place: None,
                agent: AgentKind::Claude,
            },
        },
    )
    .unwrap();
    drop(leaving);
    for _ in 0..600 {
        if get(&mut connection, later).workspace.is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("the Start did not finish without its requester");
}

/// ADR 0068: a data directory whose path an npm `.cmd` shim refuses fails the agent step
/// with a message saying so, and the Tab it opened closes.
#[test]
fn a_data_directory_a_cmd_shim_refuses_fails_the_agent_step() {
    let fixture = Fixture::in_data("shim", "R&D (100%)");
    fixture.agents(vec![AgentInstallation {
        kind: AgentKind::Claude,
        executable: fixture.root.join("claude.cmd"),
    }]);
    let mut connection = fixture.connect();
    let folder = fixture.root.join("notes");
    std::fs::create_dir_all(&folder).unwrap();
    let workspace = fixture.workspace(&mut connection, &folder);
    let task = new_task(&mut connection, "Shim", None);
    let failed = start_task(&mut connection, task, None).unwrap_err();
    assert_eq!(failed.code, task_error::UNSUPPORTED_PATH);
    assert_eq!(failed.step.as_deref(), Some("agent"));
    assert!(failed.message.contains(".cmd shim"));
    assert_eq!(
        fixture.session().workspace(workspace).unwrap().tabs().len(),
        1
    );
    assert_eq!(get(&mut connection, task).workspace, None);
}

/// Waits until `done` holds of the Session.
fn wait_for_session(fixture: &Fixture, done: impl Fn(&Session) -> bool) {
    for _ in 0..400 {
        if done(&fixture.session()) {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("the Session never got there");
}

fn archive(
    connection: &mut ClientConnection,
    task: TaskId,
    remove_worktree: bool,
) -> Result<TaskResponse, TaskError> {
    connection
        .task(TaskCommand::Archive {
            task,
            close_workspace: true,
            remove_worktree,
        })
        .unwrap()
}

/// ADR 0068: archiving a started Task closes its Workspace when asked and, for a clean
/// Managed Worktree, removes it while its branch stays; a worktree with changes, or a
/// Workspace that is no Managed Worktree, refuses the removal and keeps the Task started.
#[test]
fn archiving_closes_the_workspace_and_removes_a_clean_managed_worktree() {
    let fixture = Fixture::new("archive");
    let args = fixture.agent(AgentKind::Claude, "claude");
    let mut connection = fixture.connect();
    let (repository, _) = fixture.repository(&mut connection);
    let task = new_task(&mut connection, "Fix the login bug", Some(&repository));
    let (workspace, _, _) = start_task(
        &mut connection,
        task,
        Some(StartPlace::NewWorktree { branch: None }),
    )
    .unwrap();
    agent_args(&args);
    let folder = fixture
        .session()
        .workspace(workspace)
        .unwrap()
        .root_directory()
        .to_path_buf();

    std::fs::write(folder.join("notes.txt"), "keep me\n").unwrap();
    assert_eq!(
        archive(&mut connection, task, true).unwrap_err().code,
        task_error::WORKTREE_NOT_REMOVABLE
    );
    let still = get(&mut connection, task);
    assert!(!still.archived && still.workspace == Some(workspace));

    std::fs::remove_file(folder.join("notes.txt")).unwrap();
    let Ok(TaskResponse::Task(archived)) = archive(&mut connection, task, true) else {
        panic!("archived")
    };
    assert!(archived.archived && archived.workspace.is_none());
    assert!(!fixture.file(task).exists());
    wait_for_session(&fixture, |session| session.workspace(workspace).is_none());
    for _ in 0..200 {
        if !folder.exists() {
            break;
        }
        thread::sleep(Duration::from_millis(50));
    }
    assert!(!folder.exists(), "the worktree is removed");
    assert!(branch_exists(&repository, "worktree/fix-the-login-bug"));

    // A plain Workspace closes, and its folder stays.
    let notes = fixture.root.join("notes");
    std::fs::create_dir_all(&notes).unwrap();
    let plain = fixture.workspace(&mut connection, &notes);
    let other = new_task(&mut connection, "Write the notes", None);
    start_task(&mut connection, other, Some(StartPlace::Workspace(plain))).unwrap();
    assert_eq!(
        archive(&mut connection, other, true).unwrap_err().code,
        task_error::WORKTREE_NOT_REMOVABLE,
        "only a Managed Worktree is removed"
    );
    assert!(matches!(
        archive(&mut connection, other, false),
        Ok(TaskResponse::Task(task)) if task.archived
    ));
    wait_for_session(&fixture, |session| session.workspace(plain).is_none());
    assert!(notes.is_dir());
}

/// ADR 0068: a Server that stops in the middle of a Start loses the reservation, and the
/// Workspace it had made stays, without a Task.
#[test]
fn a_server_stopped_mid_start_keeps_the_workspace_without_a_task() {
    let mut fixture = Fixture::new("stopped");
    fixture.agent(AgentKind::Claude, "claude");
    let mut connection = fixture.connect();
    let (repository, _) = fixture.repository(&mut connection);
    let task = new_task(&mut connection, "Never typed", Some(&repository));
    // The new shell never looks ready, so the Start waits until the Server stops.
    fixture
        .handle
        .state
        .lock()
        .unwrap()
        .tasks
        .as_mut()
        .unwrap()
        .shells_busy = true;

    let server_id = connection.overview().server_id;
    let mut stream = connection.into_stream();
    condr_core::protocol::write_message(
        &mut stream,
        &ClientMessage::Task {
            server_id,
            request_id: 1,
            command: TaskCommand::Start {
                task,
                place: Some(StartPlace::NewWorktree { branch: None }),
                agent: AgentKind::Claude,
            },
        },
    )
    .unwrap();
    wait_for_session(&fixture, |session| session.workspaces().len() == 2);
    let folder = fixture.session().workspaces()[1]
        .root_directory()
        .to_path_buf();

    fixture.handle.stop();
    fixture.thread.take().unwrap().join().unwrap().unwrap();
    let answer = loop {
        match read_server(&mut stream) {
            ServerMessage::Task { result, .. } => break result,
            _ => continue,
        }
    };
    assert_eq!(answer.unwrap_err().code, task_error::SERVER_STOPPING);
    assert!(folder.is_dir(), "the Workspace it made stays");
    assert!(branch_exists(&repository, "worktree/never-typed"));
    let stored = crate::task_store::TaskStore::open(&fixture.root.join("data/tasks.sqlite"))
        .unwrap()
        .tasks;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].workspace, None, "no Task holds it");
}
