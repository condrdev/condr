//! The Server's Tasks (ADR 0068): the requests, the link to a Workspace, who receives
//! them, and `StartTask`.

use super::*;
use condr_core::protocol::Hello;
use condr_core::{
    AgentKind, LastPlace, StartPlace, Task, TaskCommand, TaskError, TaskId, TaskResponse,
    task_error,
};

fn temp(name: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("condr-tasks-{name}-{}", unique_suffix()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn create(state: &mut RuntimeState, title: &str) -> Task {
    match state.task_command(TaskCommand::Create {
        title: title.into(),
        description: format!("{title}: the details"),
        repository: None,
    }) {
        Ok(TaskResponse::Task(task)) => task,
        other => panic!("{other:?}"),
    }
}

fn listed(state: &mut RuntimeState, archived: bool) -> Vec<TaskId> {
    match state.task_command(TaskCommand::List { archived }) {
        Ok(TaskResponse::Tasks(tasks)) => {
            assert!(tasks.iter().all(|task| task.description.is_none()));
            tasks.into_iter().map(|task| task.id).collect()
        }
        other => panic!("{other:?}"),
    }
}

fn code(result: Result<TaskResponse, TaskError>) -> String {
    result.expect_err("refused").code
}

/// ADR 0068: create, edit, order, archive, restore and delete, each told to Clients as
/// one event for that Task, without its description.
#[test]
fn task_requests_create_edit_order_archive_restore_and_delete() {
    let mut state = RuntimeState::new(test_endpoint().as_local_path().unwrap());
    let a = create(&mut state, "A").id;
    let b = create(&mut state, "B").id;
    let c = create(&mut state, "C").id;
    assert_eq!(listed(&mut state, false), [c, b, a], "new ones on top");

    for refused in [
        TaskCommand::Create {
            title: "  ".into(),
            description: String::new(),
            repository: None,
        },
        TaskCommand::Create {
            title: "two\nlines".into(),
            description: String::new(),
            repository: None,
        },
        TaskCommand::Create {
            title: "t".into(),
            description: "x".repeat(condr_core::MAX_TASK_DESCRIPTION_BYTES + 1),
            repository: None,
        },
        TaskCommand::Create {
            title: "t".into(),
            description: String::new(),
            repository: Some("/nowhere/open".into()),
        },
    ] {
        assert_eq!(code(state.task_command(refused)), task_error::INVALID_TASK);
    }

    let edited = state.task_command(TaskCommand::Edit {
        task: a,
        title: Some(" A, renamed ".into()),
        description: None,
        repository: None,
    });
    let Ok(TaskResponse::Task(edited)) = edited else {
        panic!("{edited:?}")
    };
    assert_eq!(edited.title, "A, renamed");
    assert_eq!(edited.description.as_deref(), Some("A: the details"));

    state
        .task_command(TaskCommand::Reorder { task: a, index: 0 })
        .unwrap();
    assert_eq!(listed(&mut state, false), [a, c, b]);
    state
        .task_command(TaskCommand::Reorder { task: a, index: 9 })
        .unwrap();
    assert_eq!(listed(&mut state, false), [c, b, a]);

    state
        .task_command(TaskCommand::Archive {
            task: b,
            close_workspace: false,
            remove_worktree: false,
        })
        .unwrap();
    assert_eq!(listed(&mut state, false), [c, a]);
    assert_eq!(listed(&mut state, true), [b]);
    assert_eq!(
        code(state.task_command(TaskCommand::Reorder { task: b, index: 0 })),
        task_error::TASK_ARCHIVED
    );
    state
        .task_command(TaskCommand::Restore { task: b })
        .unwrap();
    assert_eq!(listed(&mut state, false), [b, c, a], "restored on top");

    // A started Task is deleted only once archived.
    let workspace = state
        .session
        .create_workspace(std::env::temp_dir())
        .unwrap();
    state.task_mut(c).unwrap().workspace = Some(workspace);
    for command in [
        TaskCommand::Delete { task: c },
        TaskCommand::Reorder { task: c, index: 0 },
    ] {
        assert_eq!(code(state.task_command(command)), task_error::TASK_STARTED);
    }
    state.task_mut(a).unwrap().starting = true;
    assert_eq!(
        code(state.task_command(TaskCommand::Delete { task: a })),
        task_error::TASK_STARTING
    );
    state.task_mut(a).unwrap().starting = false;

    let events_before = state.events.len();
    assert_eq!(
        state.task_command(TaskCommand::Delete { task: a }),
        Ok(TaskResponse::Done)
    );
    assert_eq!(
        code(state.task_command(TaskCommand::Get { task: a })),
        task_error::TASK_NOT_FOUND
    );
    assert!(matches!(
        state.events.back().map(|event| &event.event),
        Some(SessionEvent::TaskRemoved { task }) if *task == a
    ));
    assert_eq!(state.events.len(), events_before + 1);
    let Ok(TaskResponse::Task(full)) = state.task_command(TaskCommand::Get { task: b }) else {
        panic!("Get answers with the Task")
    };
    assert_eq!(full.description.as_deref(), Some("B: the details"));
    assert!(state.events.iter().all(|event| match &event.event {
        SessionEvent::TaskChanged(task) => task.description.is_none(),
        _ => true,
    }));
}

/// ADR 0068: a Workspace that closes lets go of its Task in the same step, which keeps
/// the place it was worked.
#[test]
fn a_closing_workspace_clears_its_tasks_link_and_records_the_last_place() {
    let mut state = RuntimeState::new(test_endpoint().as_local_path().unwrap());
    let root = std::env::temp_dir();
    let parent = state.session.create_workspace(root.join("parent")).unwrap();
    let workspace = state.session.create_workspace(root.join("child")).unwrap();
    assert!(state.session.associate_worktree(
        workspace,
        parent,
        root.join("parent"),
        true,
        Some("main".into()),
    ));
    let task = create(&mut state, "Fix it").id;
    state.task_mut(task).unwrap().workspace = Some(workspace);

    let effect = apply_layout_command(
        &mut state,
        LayoutCommand::CloseWorkspace {
            workspace_id: workspace,
        },
    )
    .unwrap();
    let Ok(TaskResponse::Task(task)) = state.task_command(TaskCommand::Get { task }) else {
        panic!("the Task stays")
    };
    assert_eq!(task.workspace, None);
    assert_eq!(
        task.last,
        Some(LastPlace {
            path: root.join("child"),
            branch: None,
            base_branch: Some("main".into()),
            managed: true,
            repository: Some(root.join("parent")),
        })
    );
    assert!(matches!(
        state.events.back().map(|event| &event.event),
        Some(SessionEvent::TaskChanged(changed)) if changed.workspace.is_none()
    ));
    assert!(matches!(effect.event, SessionEvent::LayoutChanged { .. }));
}

/// ADR 0068: Tasks come back after a restart, and one whose Workspace did not keeps the
/// last place its Start recorded.
#[test]
fn tasks_survive_a_restart_and_one_whose_workspace_did_not_returns_to_todo() {
    let data = temp("restart");
    let socket = test_endpoint();
    let recover = || {
        RuntimeState::recover(
            socket.as_local_path().unwrap(),
            None,
            None,
            Some(data.clone()),
        )
        .unwrap()
        .0
    };
    let last = LastPlace {
        path: data.join("worktree"),
        branch: Some("worktree/fix-it".into()),
        base_branch: None,
        managed: false,
        repository: None,
    };
    let (kept, linked) = {
        let mut state = recover();
        let kept = create(&mut state, "Kept").id;
        let linked = create(&mut state, "Linked").id;
        let task = state.task_mut(linked).unwrap();
        task.workspace = Some(WorkspaceId::from_u64(987_654));
        task.last = Some(last.clone());
        state.task_changed(linked);
        state.tasks.as_mut().unwrap().shutdown().unwrap();
        (kept, linked)
    };
    let mut state = recover();
    assert_eq!(listed(&mut state, false), [linked, kept]);
    let Ok(TaskResponse::Task(task)) = state.task_command(TaskCommand::Get { task: linked }) else {
        panic!("the Task came back")
    };
    assert_eq!(task.workspace, None, "its Workspace did not come back");
    assert_eq!(task.last, Some(last));
    assert_eq!(task.description.as_deref(), Some("Linked: the details"));
    drop(state);
    let _ = std::fs::remove_dir_all(data);
}

/// ADR 0068: a task store the Server cannot read is set aside, and it starts with no
/// Tasks and keeps new ones.
#[test]
fn a_server_with_an_unreadable_task_store_starts_with_no_tasks() {
    let data = temp("unreadable");
    std::fs::write(data.join("tasks.sqlite"), "not a database").unwrap();
    let mut state = RuntimeState::recover(
        test_endpoint().as_local_path().unwrap(),
        None,
        None,
        Some(data.clone()),
    )
    .unwrap()
    .0;
    assert!(listed(&mut state, false).is_empty());
    assert!(
        std::fs::read_dir(&data).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("tasks.sqlite.unreadable-")),
        "the old file is left for the person"
    );
    let task = create(&mut state, "New").id;
    assert_eq!(listed(&mut state, false), [task]);
    drop(state);
    let _ = std::fs::remove_dir_all(data);
}

fn start_with(
    config: ServerConfig,
) -> (ServerHandle, Endpoint, thread::JoinHandle<io::Result<()>>) {
    let endpoint = config.local_endpoint();
    let server = BoundServer::bind(config).unwrap();
    let handle = server.handle();
    let thread = thread::spawn(move || server.run());
    wait_for(&endpoint);
    (handle, endpoint, thread)
}

fn wait_for(endpoint: &Endpoint) {
    for _ in 0..200 {
        if endpoint.connect().is_ok() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("server did not start");
}

fn tasks_server(
    data: &std::path::Path,
) -> (ServerHandle, Endpoint, thread::JoinHandle<io::Result<()>>) {
    start_with(
        ServerConfig::ephemeral(test_endpoint().as_local_path().unwrap()).with_data_directory(data),
    )
}

fn stop(handle: ServerHandle, thread: thread::JoinHandle<io::Result<()>>) {
    handle.stop();
    thread.join().unwrap().unwrap();
}

fn hinted() -> Hello {
    Hello {
        tasks: true,
        ..Hello::new("test")
    }
}

fn task_of(connection: &mut ClientConnection, command: TaskCommand) -> Task {
    match connection.task(command).unwrap() {
        Ok(TaskResponse::Task(task)) => task,
        other => panic!("{other:?}"),
    }
}

/// Reads up to the Event with `sequence`, skipping terminal frames.
fn event_at(stream: &mut EndpointStream, sequence: u64) -> SessionEvent {
    match wait_for_message(
        stream,
        |message| matches!(message, ServerMessage::Event { sequence: at, .. } if *at == sequence),
    ) {
        ServerMessage::Event { event, .. } => event,
        _ => unreachable!(),
    }
}

/// ADR 0068: Clients that said `tasks` receive the same Tasks through the Bootstrap and
/// per-Task events, without descriptions; one that did not receives none, only `Omitted`
/// under the same sequence. Every Client edits, and the last save wins.
#[test]
fn only_clients_that_said_tasks_receive_them_and_the_last_save_wins() {
    let data = temp("hint");
    let (handle, endpoint, thread) = tasks_server(&data);
    let mut writer = ClientConnection::connect(&endpoint, "writer").unwrap();
    let first = task_of(
        &mut writer,
        TaskCommand::Create {
            title: "First".into(),
            description: "secret details".into(),
            repository: None,
        },
    );

    let mut subscribed = Vec::new();
    for hello in [hinted(), hinted(), Hello::new("old")] {
        let takes_tasks = hello.tasks;
        let connection = ClientConnection::connect(&endpoint, hello).unwrap();
        let bootstrap = connection.bootstrap().unwrap().clone();
        if takes_tasks {
            assert_eq!(bootstrap.tasks, Some(vec![first.summary()]));
        } else {
            assert_eq!(bootstrap.tasks, None);
        }
        let mut stream = connection.into_stream();
        subscribe(&mut stream, bootstrap.session_id, bootstrap.sequence);
        subscribed.push((stream, bootstrap.sequence));
    }
    let second = task_of(
        &mut writer,
        TaskCommand::Create {
            title: "Second".into(),
            description: "more".into(),
            repository: None,
        },
    );
    task_of(
        &mut writer,
        TaskCommand::Edit {
            task: second.id,
            title: Some("Second, by one".into()),
            description: None,
            repository: None,
        },
    );
    task_of(
        &mut writer,
        TaskCommand::Edit {
            task: second.id,
            title: Some("Second, by another".into()),
            description: None,
            repository: None,
        },
    );
    for (index, (stream, sequence)) in subscribed.iter_mut().enumerate() {
        let events = (1..=3)
            .map(|offset| event_at(stream, *sequence + offset))
            .collect::<Vec<_>>();
        if index < 2 {
            let titles = events
                .iter()
                .map(|event| match event {
                    SessionEvent::TaskChanged(task) => {
                        assert_eq!(task.description, None);
                        task.title.clone()
                    }
                    other => panic!("{other:?}"),
                })
                .collect::<Vec<_>>();
            assert_eq!(titles, ["Second", "Second, by one", "Second, by another"]);
        } else {
            assert!(events.iter().all(|event| *event == SessionEvent::Omitted));
        }
    }
    let last = task_of(&mut writer, TaskCommand::Get { task: second.id });
    assert_eq!(last.title, "Second, by another");
    assert_eq!(last.description.as_deref(), Some("more"));
    drop(subscribed);
    stop(handle, thread);
    let _ = std::fs::remove_dir_all(data);
}

/// ADR 0068: a Server without Tasks, as an older one, sends no Tasks record to a Client
/// that asks and answers a Task request with the frozen `UNSUPPORTED_REQUEST`, which the
/// CLI turns into `feature_unavailable`.
#[test]
fn a_server_without_tasks_answers_as_an_older_one() {
    let (handle, endpoint, thread) = start();
    handle.state.lock().unwrap().tasks = None;
    let mut connection = ClientConnection::connect(&endpoint, hinted()).unwrap();
    assert_eq!(connection.bootstrap().unwrap().tasks, None);
    let refused = connection
        .task(TaskCommand::List { archived: false })
        .unwrap_err();
    assert_eq!(
        refused.to_string(),
        condr_core::protocol::UNSUPPORTED_REQUEST
    );
    stop(handle, thread);
}

#[test]
fn a_cmd_shim_refuses_a_task_file_path_with_its_metacharacters() {
    let shim = condr_core::agent_discovery::AgentInstallation {
        kind: AgentKind::Codex,
        executable: PathBuf::from(r"C:\Users\me\AppData\Roaming\npm\codex.cmd"),
    };
    let exe = condr_core::agent_discovery::AgentInstallation {
        executable: PathBuf::from(r"C:\Tools\codex.exe"),
        ..shim.clone()
    };
    let odd = std::path::Path::new(r"C:\Users\R&D (100%)\condr\tasks\1.md");
    let plain = std::path::Path::new(r"C:\Users\me\condr\tasks\1.md");
    let refused = task_start::check_shim(&shim, odd).unwrap_err();
    assert_eq!(refused.code, task_error::UNSUPPORTED_PATH);
    assert!(refused.message.contains(".cmd shim"), "{}", refused.message);
    assert!(task_start::check_shim(&shim, plain).is_ok());
    assert!(task_start::check_shim(&exe, odd).is_ok());
    assert_eq!(
        task_start::initial_prompt(plain),
        r"Read C:\Users\me\condr\tasks\1.md and do the task it describes."
    );
}

#[cfg(unix)]
mod start;
