//! The Task form: saving, leaving with changes, another Client's changes, the
//! description it loads, and moving a To do Task to another Device.

use super::*;

#[test]
fn a_new_task_saves_with_the_shortcut_and_unsaved_changes_ask_before_they_go() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, mut cli) = tasks_gui(&mut cx);
    // A Workspace to start in, so Start waits only for the changes.
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                name: None,
                root_directory: std::env::temp_dir(),
            });
        });
    });
    assert!(wait_until(window, |window| presented(window, &view).is_some()));
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    draw(window);

    // A new Task has no card and no Start until it is saved.
    click(window, "tasks-new-task");
    assert!(window.debug_bounds("task-form").is_some());
    assert!(window.debug_bounds("task-start-unsaved").is_some());
    assert!(window.debug_bounds("task-start").is_none());
    assert!(gui_tasks(window, &view).is_empty());
    window.simulate_input("Write the release notes");
    click(window, "task-form-save");
    assert!(wait_until(window, |window| form_home(window, &view).is_some()));
    draw(window);
    let (_, saved) = form_home(window, &view).unwrap();
    assert_eq!(
        gui_tasks(window, &view)[&saved].title,
        "Write the release notes"
    );
    assert!(inside(window, todo_card(saved), "tasks-column-to-do"));
    assert!(window.debug_bounds("task-start").is_some());
    assert!(window.debug_bounds("task-start-blocked").is_none());

    // A change disables Start until it is saved.
    click(window, "task-form-title");
    window.simulate_input(" now");
    draw(window);
    assert!(window.debug_bounds("task-start-blocked").is_some());

    // Leaving asks: Cancel stays, Discard goes and keeps the Task as it was.
    let other = create_task(&mut cli, "Fix the login redirect", None);
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 2));
    draw(window);
    click(window, todo_card(other));
    assert!(window.update(|window, cx| window.has_active_dialog(cx)));
    click(window, "file-choice-cancel");
    assert_eq!(form_home(window, &view), Some((1, saved)));
    click(window, todo_card(other));
    click(window, "file-choice-discard");
    assert_eq!(form_home(window, &view), Some((1, other)));
    assert_eq!(
        gui_tasks(window, &view)[&saved].title,
        "Write the release notes"
    );

    // The form fetched the description; Save from the dialog keeps a change on the way out.
    assert!(wait_until(window, |window| window.read(|app| {
        view.read(app)
            .task_form
            .as_ref()
            .and_then(|form| form.saved.as_ref())
            .is_some_and(|saved| saved.description.is_some())
    })));
    click(window, "task-form-title");
    window.simulate_input("!");
    click(window, "tasks-area-close");
    click(window, "file-choice-save");
    // The form goes once its save is in.
    assert!(wait_until(window, |window| window
        .read(|app| view.read(app).task_form.is_none())));
    assert!(wait_until(window, |window| gui_tasks(window, &view)
        [&other]
        .title
        .ends_with('!')));
    let Ok(Ok(TaskResponse::Task(task))) = cli.task(TaskCommand::Get { task: other }) else {
        panic!("the Task is there");
    };
    assert_eq!(
        task.description.as_deref(),
        Some("Fix the login redirect, described"),
        "saving a title leaves the description as it was"
    );

    // Archive asks nothing; Delete asks first.
    click(window, todo_card(saved));
    click(window, "task-form-delete");
    assert!(window.update(|window, cx| window.has_active_dialog(cx)));
    window.update(|window, cx| window.dispatch_action(Box::new(Confirm { secondary: false }), cx));
    assert!(wait_until(window, |window| !gui_tasks(window, &view)
        .contains_key(&saved)));
    click(window, todo_card(other));
    click(window, "task-form-archive");
    assert!(wait_until(window, |window| gui_tasks(window, &view).is_empty()));
    assert!(window.read(|app| view.read(app).task_form.is_none()));

    // The Archived list unfolds with it, to restore.
    click(window, "tasks-archived");
    let restore = leaked_selector(format!("tasks-restore-1-{}", other));
    assert!(wait_until(window, |window| {
        draw(window);
        window.debug_bounds(restore).is_some()
    }));
    click(window, restore);
    assert!(wait_until(window, |window| gui_tasks(window, &view)
        .contains_key(&other)));
}

#[test]
fn a_form_closes_when_another_client_deletes_its_task_and_follows_one_that_starts() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, mut cli) = tasks_gui(&mut cx);
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                name: None,
                root_directory: std::env::temp_dir(),
            });
        });
    });
    assert!(wait_until(window, |window| presented(window, &view).is_some()));
    let (workspace_id, _) = presented(window, &view).unwrap();
    let deleted = create_task(&mut cli, "Fix the login redirect", None);
    let started = create_task(&mut cli, "Write the release notes", None);
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 2));
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    draw(window);

    click(window, todo_card(deleted));
    assert_eq!(form_home(window, &view), Some((1, deleted)));
    let Ok(Ok(_)) = cli.task(TaskCommand::Delete { task: deleted }) else {
        panic!("the other Client deletes it");
    };
    assert!(wait_until(window, |window| form_home(window, &view).is_none()));

    // Another Client starts it: the selection follows to its card, which reads
    // "Starting…" in Working until an Agent comes, and then "No agent running".
    click(window, todo_card(started));
    let mut task = gui_tasks(window, &view)[&started].clone();
    task.workspace = Some(workspace_id);
    event(window, &view, SessionEvent::TaskChanged(task));
    let card = leaked_selector(format!("tasks-task-1-{}", started));
    assert!(window.read(|app| view.read(app).task_form.is_none()));
    assert_eq!(
        window.read(|app| view.read(app).main_view().tasks_selection.unwrap().task),
        Some(started)
    );
    assert!(inside(window, card, "tasks-column-working"));
    assert!(window.debug_bounds("tasks-archive-task").is_some());
    assert!(window.debug_bounds("tasks-area-dock").is_some());

    window.update(|_, cx| view.update(cx, |this, cx| this.task_start_settled(1, started, cx)));
    draw(window);
    assert!(inside(window, card, "tasks-column-idle"));
    assert!(
        window.read(|app| view.read(app).last_error.is_none()),
        "only the Client that started it says the Agent did not come"
    );
}

#[test]
fn a_new_device_for_a_to_do_task_creates_it_there_and_deletes_it_here() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, mut cli) = tasks_gui(&mut cx);
    let task_id = create_task(&mut cli, "Fix the login redirect", None);
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 1));
    // A second Device that keeps Tasks, whose messages the test answers.
    let (outgoing, box_sent) = std::sync::mpsc::channel();
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            let mut device =
                ServerConnection::new(2, "box".into(), crate::app::tests::tcp("127.0.0.1:9"));
            device.status = ConnectionStatus::Connected;
            device.model.server_id = Some(condr_core::protocol::ServerId(77));
            device.model.session_id = Some(condr_core::protocol::SessionId(1));
            device.model.tasks = Some(Default::default());
            device.io = Some(ClientIo {
                outgoing,
                _incoming_task: Task::ready(()),
            });
            this.connections.push(device);
            this.toggle_tasks(window, cx);
        })
    });
    draw(window);
    click(window, todo_card(task_id));
    assert!(wait_until(window, |window| window.read(|app| {
        view.read(app)
            .task_form
            .as_ref()
            .and_then(|form| form.saved.as_ref())
            .is_some_and(|saved| saved.description.is_some())
    })));
    window.update(|_, cx| view.update(cx, |this, cx| this.choose_task_device(2, cx)));
    draw(window);
    assert!(
        window.debug_bounds("task-start-blocked").is_some(),
        "a move is a change to save before Start"
    );
    click(window, "task-form-save");
    let to_box: Vec<ClientMessage> = box_sent.try_iter().collect();
    assert!(
        to_box.iter().any(|message| matches!(
            message,
            ClientMessage::Agent {
                command: condr_core::protocol::AgentCommand::Available,
                ..
            }
        )),
        "the Device the Task moves to is asked for its Agents"
    );
    let (request_id, title, description) = to_box
        .into_iter()
        .find_map(|message| match message {
            ClientMessage::Task {
                request_id,
                command:
                    TaskCommand::Create {
                        title,
                        description,
                        repository: None,
                    },
                ..
            } => Some((request_id, title, description)),
            _ => None,
        })
        .expect("the new Device is asked to create it");
    assert_eq!(description, "Fix the login redirect, described");
    let moved = condr_core::Task {
        id: TaskId::from_u128(5),
        title,
        description: Some(description),
        repository: None,
        order: 0,
        archived: false,
        workspace: None,
        made_workspace: false,
        starting: false,
        last: None,
    };
    let answer = |window: &mut VisualTestContext, request_id, result| {
        window.update(|_, cx| {
            view.update(cx, |this, cx| {
                this.handle_incoming(
                    2,
                    0,
                    Incoming::Message(ServerMessage::Task { request_id, result }),
                    cx,
                );
            })
        });
        window.run_until_parked();
    };
    // Its event comes before its answer, as the Server sends them.
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.connection_mut(2)
                .unwrap()
                .model
                .apply_task(moved.clone());
        })
    });
    answer(window, request_id, Ok(TaskResponse::Task(moved)));
    assert_eq!(form_home(window, &view), Some((2, TaskId::from_u128(5))));
    assert!(
        wait_until(window, |window| gui_tasks(window, &view).is_empty()),
        "then the old Device deletes its copy"
    );

    // From a Device whose delete fails: both copies stay, and a toast says so.
    let stay = |window: &mut VisualTestContext, id: u128| {
        let mut task = moved_task(id);
        task.description = None;
        window.update(|_, cx| {
            view.update(cx, |this, _| {
                this.connection_mut(2).unwrap().model.apply_task(task);
            })
        });
        window.update(|window, cx| {
            view.update(cx, |this, cx| {
                this.open_task_form(2, TaskId::from_u128(id), window, cx)
            })
        });
        let get = box_sent
            .try_iter()
            .find_map(|message| match message {
                ClientMessage::Task {
                    request_id,
                    command: TaskCommand::Get { .. },
                    ..
                } => Some(request_id),
                _ => None,
            })
            .expect("the form fetches its description");
        answer(window, get, Ok(TaskResponse::Task(moved_task(id))));
        window.update(|_, cx| view.update(cx, |this, cx| this.choose_task_device(1, cx)));
    };
    stay(window, 7);
    click(window, "task-form-save");
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 1));
    let delete = box_sent
        .try_iter()
        .find_map(|message| match message {
            ClientMessage::Task {
                request_id,
                command: TaskCommand::Delete { task },
                ..
            } if task == TaskId::from_u128(7) => Some(request_id),
            _ => None,
        })
        .expect("the old Device is asked to delete its copy");
    answer(
        window,
        delete,
        Err(condr_core::TaskError::new(
            condr_core::task_error::TASK_STARTED,
            "Task 7 is started; archive it first",
        )),
    );
    assert!(window.read(|app| view.read(app).last_error.is_some()));
    let both = window.read(|app| {
        view.read(app)
            .connection(2)
            .unwrap()
            .model
            .tasks
            .as_ref()
            .unwrap()
            .contains_key(&TaskId::from_u128(7))
    });
    assert!(both, "the old copy stays beside the new one");

    // An old Device gone before the new copy exists: nothing to delete it on, said so.
    window.update(|_, cx| view.update(cx, |this, _| this.last_error = None));
    stay(window, 8);
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.connection_mut(2).unwrap().status = ConnectionStatus::Disconnected;
        })
    });
    click(window, "task-form-save");
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 2));
    window.run_until_parked();
    assert!(window.read(|app| view.read(app).last_error.is_some()));
}

#[test]
fn a_start_answer_never_takes_a_form_that_would_lose_text() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, mut cli) = tasks_gui(&mut cx);
    // Two Workspaces, one for each Start.
    for index in 0..2 {
        let root_directory = std::env::temp_dir().join(format!("condr-tasks-answer-{index}"));
        std::fs::create_dir_all(&root_directory).unwrap();
        window.update(|_, cx| {
            view.update(cx, |this, _| {
                this.send_layout(LayoutCommand::CreateWorkspace {
                    name: None,
                    root_directory,
                });
            });
        });
        assert!(wait_until(window, |window| window.read(|app| {
            view.read(app)
                .active_session()
                .is_some_and(|session| session.workspaces().len() > index)
        })));
    }
    let workspaces = window.read(|app| {
        view.read(app)
            .active_session()
            .unwrap()
            .workspaces()
            .iter()
            .map(|workspace| workspace.id())
            .collect::<Vec<_>>()
    });
    let started = create_task(&mut cli, "Fix the login redirect", None);
    let other = create_task(&mut cli, "Write the release notes", None);
    let own = create_task(&mut cli, "Update the changelog", None);
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 3));
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    draw(window);
    let sent = capture(window, &view);
    // The form's description, answered by hand since the Server no longer hears it.
    let open = |window: &mut VisualTestContext, task: TaskId| {
        click(window, todo_card(task));
        let request_id = sent
            .try_iter()
            .find_map(|message| match message {
                ClientMessage::Task {
                    request_id,
                    command: TaskCommand::Get { .. },
                    ..
                } => Some(request_id),
                _ => None,
            })
            .expect("the form fetches its description");
        let mut full = gui_tasks(window, &view)[&task].clone();
        full.description = Some(String::new());
        incoming(
            window,
            &view,
            ServerMessage::Task {
                request_id,
                result: Ok(TaskResponse::Task(full)),
            },
        );
    };
    let start = |window: &mut VisualTestContext, task: TaskId| {
        open(window, task);
        window.update(|_, cx| {
            view.update(cx, |this, cx| {
                this.connection_mut(1).unwrap().device_tasks.available =
                    Some(vec![AgentKind::Claude]);
                cx.notify();
            })
        });
        draw(window);
        click(window, "task-start-button");
        sent.try_iter()
            .find_map(|message| match message {
                ClientMessage::Task {
                    request_id,
                    command: TaskCommand::Start { .. },
                    ..
                } => Some(request_id),
                _ => None,
            })
            .expect("Start is sent")
    };
    let started_answer = |window: &mut VisualTestContext,
                          task: TaskId,
                          workspace_id: WorkspaceId,
                          request_id: u64| {
        let mut record = gui_tasks(window, &view)[&task].clone();
        record.workspace = Some(workspace_id);
        event(window, &view, SessionEvent::TaskChanged(record));
        let (tab_id, pane_id) = window.read(|app| {
            let tab = &view
                .read(app)
                .active_session()
                .unwrap()
                .workspace(workspace_id)
                .unwrap()
                .tabs()[0];
            (tab.id(), tab.panes()[0].id())
        });
        incoming(
            window,
            &view,
            ServerMessage::Task {
                request_id,
                result: Ok(TaskResponse::Started {
                    workspace_id,
                    tab_id,
                    pane_id,
                }),
            },
        );
    };

    // Another Task's form, with text typed, outlives the Start's answer.
    let request = start(window, started);
    open(window, other);
    set_title(window, &view, "Write the release notes today");
    started_answer(window, started, workspaces[0], request);
    assert_eq!(form_home(window, &view), Some((1, other)));
    assert_eq!(form_text(window, &view).0, "Write the release notes today");
    assert!(window.read(|app| view.read(app).main_view().tasks_selection.is_none()));
    assert!(window.read(|app| view.read(app).task_starting(1, started)));

    // As does the started Task's own form with a change: it stays to be saved, and
    // Start says it is under way.
    set_title(window, &view, "Write the release notes");
    let request = start(window, own);
    set_title(window, &view, "Update the changelog for 0.3");
    started_answer(window, own, workspaces[1], request);
    assert_eq!(form_home(window, &view), Some((1, own)));
    assert_eq!(form_text(window, &view).0, "Update the changelog for 0.3");
    assert!(window.debug_bounds("task-start-blocked").is_some());
}

#[test]
fn the_leave_dialogs_save_keeps_the_form_until_its_save_is_in() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, mut cli) = tasks_gui(&mut cx);
    let task = create_task(&mut cli, "Fix the login redirect", None);
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 1));
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    draw(window);
    open_form(window, &view, task);
    let sent = capture(window, &view);
    use crate::app::tasks::FormField;

    // Saved on the way out and refused: the form stays, with the refusal, where it was.
    set_title(window, &view, "Fix the login redirect!");
    click(window, "tasks-area-close");
    click(window, "file-choice-save");
    assert_eq!(
        form_home(window, &view),
        Some((1, task)),
        "waiting for the answer"
    );
    let request_id = sent
        .try_iter()
        .find_map(|message| match message {
            ClientMessage::Task {
                request_id,
                command: TaskCommand::Edit { .. },
                ..
            } => Some(request_id),
            _ => None,
        })
        .expect("the save is sent");
    incoming(
        window,
        &view,
        ServerMessage::Task {
            request_id,
            result: Err(condr_core::TaskError::new(
                condr_core::task_error::INVALID_TASK,
                "no repository Workspace is open at /src/api",
            )),
        },
    );
    assert_eq!(form_home(window, &view), Some((1, task)));
    assert_eq!(form_text(window, &view).0, "Fix the login redirect!");
    assert_eq!(form_error(window, &view), Some(FormField::Form));
    assert!(inside(window, "field-error", "task-form"));
    assert!(window.read(|app| view.read(app).last_error.is_none()));

    // What the Server would refuse in a title shows under it, and nothing is sent.
    set_title(window, &view, "Fix the\u{7}login redirect");
    click(window, "task-form-save");
    assert_eq!(form_error(window, &view), Some(FormField::Title));
    assert!(sent.try_iter().next().is_none());

    // A Device that is not connected keeps the form too.
    set_title(window, &view, "Fix the login redirect!");
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.connection_mut(1).unwrap().status = ConnectionStatus::Disconnected;
        })
    });
    click(window, "task-form-save");
    assert_eq!(form_error(window, &view), Some(FormField::Form));
    assert_eq!(form_home(window, &view), Some((1, task)));

    // With the form gone, a refusal is a toast that names the Task.
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.task_form = None;
            this.connection_mut(1).unwrap().device_tasks.pending.insert(
                900,
                crate::app::tasks::TaskRequest::Edit {
                    title: "Fix the login redirect".into(),
                },
            );
            cx.notify();
        })
    });
    incoming(
        window,
        &view,
        ServerMessage::Task {
            request_id: 900,
            result: Err(condr_core::TaskError::new(
                condr_core::task_error::TASK_NOT_FOUND,
                "no Task 1",
            )),
        },
    );
    assert!(
        window
            .read(|app| view.read(app).last_error.clone())
            .is_some_and(|error| error.contains("Fix the login redirect"))
    );
}

#[test]
fn a_form_loads_its_description_before_typing_and_follows_the_server_where_untouched() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, mut cli) = tasks_gui(&mut cx);
    let task = create_task(&mut cli, "Fix the login redirect", None);
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 1));
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    draw(window);

    // A fetch that fails says so and can be asked again; till then nothing is typed.
    let sent = capture(window, &view);
    click(window, todo_card(task));
    let get = |sent: &std::sync::mpsc::Receiver<ClientMessage>| {
        sent.try_iter()
            .find_map(|message| match message {
                ClientMessage::Task {
                    request_id,
                    command: TaskCommand::Get { .. },
                    ..
                } => Some(request_id),
                _ => None,
            })
            .expect("the form fetches its description")
    };
    let request_id = get(&sent);
    let loaded = |window: &mut VisualTestContext| {
        window.read(|app| {
            view.read(app)
                .task_form
                .as_ref()
                .unwrap()
                .description_loaded()
        })
    };
    assert!(!loaded(window), "the description waits disabled");
    incoming(
        window,
        &view,
        ServerMessage::Task {
            request_id,
            result: Err(condr_core::TaskError::new(
                condr_core::task_error::INTERNAL_ERROR,
                "the store is busy",
            )),
        },
    );
    assert!(window.debug_bounds("task-form-description-error").is_some());
    click(window, "task-form-retry");
    let request_id = get(&sent);
    let mut full = gui_tasks(window, &view)[&task].clone();
    full.description = Some("Users land on /home.".into());
    incoming(
        window,
        &view,
        ServerMessage::Task {
            request_id,
            result: Ok(TaskResponse::Task(full.clone())),
        },
    );
    assert!(loaded(window));
    assert!(window.debug_bounds("task-form-description-error").is_none());
    assert_eq!(form_text(window, &view).1, "Users land on /home.");

    // Another Client's change follows into the untouched description, and a title
    // saved after it leaves that change alone.
    full.description = Some("Users land on /home after signing in.".into());
    event(window, &view, SessionEvent::TaskChanged(full.clone()));
    let request_id = get(&sent);
    incoming(
        window,
        &view,
        ServerMessage::Task {
            request_id,
            result: Ok(TaskResponse::Task(full.clone())),
        },
    );
    assert_eq!(
        form_text(window, &view).1,
        "Users land on /home after signing in."
    );
    set_title(window, &view, "Fix the login redirect on Safari");
    click(window, "task-form-save");
    let edit = sent.try_iter().find_map(|message| match message {
        ClientMessage::Task {
            command: TaskCommand::Edit {
                title, description, ..
            },
            ..
        } => Some((title, description)),
        _ => None,
    });
    assert_eq!(
        edit,
        Some((Some("Fix the login redirect on Safari".into()), None)),
        "only the title is sent"
    );

    // A repository chosen on one Device comes back with it.
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.choose_task_repository(Some("/src/api".into()), cx);
            this.choose_task_device(99, cx);
        })
    });
    let repository = |window: &mut VisualTestContext| {
        window.read(|app| {
            view.read(app)
                .task_form
                .as_ref()
                .unwrap()
                .repository
                .clone()
        })
    };
    assert_eq!(repository(window), None);
    window.update(|_, cx| view.update(cx, |this, cx| this.choose_task_device(1, cx)));
    assert_eq!(repository(window), Some("/src/api".into()));
}
