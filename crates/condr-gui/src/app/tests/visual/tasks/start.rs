//! Starting a Task: its defaults, waiting for its Agent, and answers across a recovery.

use super::*;

#[test]
fn start_takes_its_defaults_and_a_successful_start_selects_the_agent() {
    let _serial_guard = acquire_visual_test_lock();
    let repository = TestDirectory::new("tasks-start");
    run_git(&repository.0, ["init"]);
    let mut cx = TestAppContext::single();
    let (view, window, _server, mut cli) = tasks_gui(&mut cx);
    let other_root = std::env::temp_dir().join("condr-tasks-start-other");
    std::fs::create_dir_all(&other_root).unwrap();
    for root in [repository.0.clone(), other_root] {
        window.update(|_, cx| {
            view.update(cx, |this, _| {
                this.send_layout(LayoutCommand::CreateWorkspace {
                    name: None,
                    root_directory: root.clone(),
                });
            });
        });
        let root = root.clone();
        assert!(wait_until(window, |window| window.read(|app| {
            view.read(app)
                .active_session()
                .is_some_and(|session| session.workspace_by_root(&root).is_some())
        })));
    }
    // A repository Workspace is one whose Git state the Server reported.
    assert!(wait_until(window, |window| window.read(|app| {
        view.read(app)
            .connection(1)
            .is_some_and(|connection| !connection.model.workspace_git.is_empty())
    })));
    let (repository_workspace, other_workspace) = window.read(|app| {
        let session = view.read(app).active_session().unwrap();
        (
            session.workspace_by_root(&repository.0).unwrap().id(),
            session.workspaces()[1].id(),
        )
    });
    let title = "Fix the login redirect";
    let task_id = create_task(&mut cli, title, Some(repository.0.clone()));
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 1));
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    draw(window);
    click(window, todo_card(task_id));
    assert!(wait_until(window, |window| window.read(|app| {
        view.read(app)
            .connection(1)
            .unwrap()
            .device_tasks
            .available
            .is_some()
    })));
    // Whatever this machine has on PATH, the Device offers one Agent here.
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.connection_mut(1).unwrap().device_tasks.available = Some(vec![AgentKind::Claude]);
            cx.notify();
        })
    });
    draw(window);
    let plan = |window: &mut VisualTestContext| {
        window.read(|app| {
            let view = view.read(app);
            let plan = view
                .start_plan(view.task_form.as_ref().unwrap(), app)
                .unwrap();
            (plan.place, plan.workspace, plan.blocked.is_some())
        })
    };
    use crate::app::tasks::Where;
    assert_eq!(
        plan(window),
        (Where::Existing, Some(repository_workspace), false),
        "the repository's own Workspace, which no Task holds"
    );

    // Another Task holds it: greyed with that Task's title, and a new worktree on the
    // Server's branch is the default.
    let mut holder = gui_tasks(window, &view)[&task_id].clone();
    holder.id = TaskId::from_u128(99);
    holder.title = "Another task".into();
    holder.workspace = Some(repository_workspace);
    put_task(window, &view, holder.clone());
    assert_eq!(plan(window).0, Where::NewWorktree);
    assert!(window.debug_bounds("task-start-branch").is_some());
    let branch = window.read(|app| {
        view.read(app)
            .task_form
            .as_ref()
            .unwrap()
            .start
            .branch
            .read(app)
            .value()
            .to_string()
    });
    assert_eq!(branch, "worktree/fix-the-login-redirect");
    let held = window.read(|app| {
        let view = view.read(app);
        view.start_plan(view.task_form.as_ref().unwrap(), app)
            .unwrap()
            .options
            .workspaces
    });
    assert_eq!(held[0].holder.as_deref(), Some("Another task"));
    assert_eq!(held[1].holder, None);

    // A branch checked out elsewhere is refused under its field.
    let sent = capture(window, &view);
    click(window, "task-start-button");
    let request_id = sent
        .try_iter()
        .find_map(|message| match message {
            ClientMessage::Task {
                request_id,
                command:
                    TaskCommand::Start {
                        task,
                        place: Some(StartPlace::NewWorktree { branch }),
                        agent: AgentKind::Claude,
                    },
                ..
            } if task == task_id
                && branch.as_deref() == Some("worktree/fix-the-login-redirect") =>
            {
                Some(request_id)
            }
            _ => None,
        })
        .expect("Start sends the place and the Agent");
    incoming(
        window,
        &view,
        ServerMessage::Task {
            request_id,
            result: Err(condr_core::TaskError::new(
                condr_core::task_error::BRANCH_CHECKED_OUT,
                "worktree/fix-the-login-redirect is checked out elsewhere",
            )),
        },
    );
    assert!(inside(window, "field-error", "task-start"));
    assert!(window.read(|app| view.read(app).last_error.is_none()));

    // The other Workspace instead; with nothing free, Start says where to open one.
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.choose_task_workspace(other_workspace, cx)
        })
    });
    assert_eq!(
        plan(window),
        (Where::Existing, Some(other_workspace), false)
    );
    let mut second = holder.clone();
    second.id = TaskId::from_u128(98);
    second.workspace = Some(other_workspace);
    put_task(window, &view, second.clone());
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let tasks = this
                .connection_mut(1)
                .unwrap()
                .model
                .tasks
                .as_mut()
                .unwrap();
            tasks.get_mut(&task_id).unwrap().repository = Some("/nowhere/cli".into());
            cx.notify();
        })
    });
    draw(window);
    assert!(window.debug_bounds("task-start-open-first").is_some());
    assert!(plan(window).2, "nothing to offer disables Start");
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let tasks = this
                .connection_mut(1)
                .unwrap()
                .model
                .tasks
                .as_mut()
                .unwrap();
            tasks.remove(&holder.id);
            tasks.remove(&second.id);
            tasks.get_mut(&task_id).unwrap().repository = Some(repository.0.clone());
            cx.notify();
        })
    });
    draw(window);

    // Start again, in the repository's Workspace: a Start under way reads "Starting…"
    // in To do.
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.choose_task_workspace(repository_workspace, cx)
        })
    });
    draw(window);
    click(window, "task-start-button");
    let request_id = sent
        .try_iter()
        .find_map(|message| match message {
            ClientMessage::Task {
                request_id,
                command:
                    TaskCommand::Start {
                        place: Some(StartPlace::Workspace(workspace)),
                        ..
                    },
                ..
            } if workspace == repository_workspace => Some(request_id),
            _ => None,
        })
        .expect("Start sends the Workspace chosen");
    let mut starting = gui_tasks(window, &view)[&task_id].clone();
    starting.starting = true;
    event(window, &view, SessionEvent::TaskChanged(starting.clone()));
    assert!(
        window
            .debug_bounds(leaked_selector(format!(
                "tasks-todo-starting-1-{}",
                task_id
            )))
            .is_some()
    );
    assert!(window.debug_bounds("task-start-blocked").is_some());

    // It started: the card is selected with the keyboard in the new Pane, and the
    // Agent's card carries the Task's title once it comes.
    let (tab_id, pane_id) = window.read(|app| {
        let tab = &view
            .read(app)
            .active_session()
            .unwrap()
            .workspace(repository_workspace)
            .unwrap()
            .tabs()[0];
        (tab.id(), tab.panes()[0].id())
    });
    let mut started = starting;
    started.starting = false;
    started.workspace = Some(repository_workspace);
    event(window, &view, SessionEvent::TaskChanged(started));
    incoming(
        window,
        &view,
        ServerMessage::Task {
            request_id,
            result: Ok(TaskResponse::Started {
                workspace_id: repository_workspace,
                tab_id,
                pane_id,
            }),
        },
    );
    assert!(window.read(|app| view.read(app).task_form.is_none()));
    let task_card = leaked_selector(format!("tasks-task-1-{}", task_id));
    assert!(inside(window, task_card, "tasks-column-working"));
    assert_eq!(target(window, &view), Some((1, pane_id)));
    // No Agent within the wait: "No agent running", and one toast for this Start.
    assert!(window.read(|app| view.read(app).last_error.is_none()));
    window
        .executor()
        .advance_clock(crate::app::tasks::AGENT_WAIT);
    window.run_until_parked();
    draw(window);
    assert!(inside(window, task_card, "tasks-column-idle"));
    assert!(window.read(|app| view.read(app).last_error.is_some()));
    agent_changed(
        window,
        &view,
        pane_id,
        Some(AgentSnapshot {
            session_id: None,
            kind: AgentKind::Claude,
            state: AgentState::Working,
            detail: None,
        }),
    );
    let agent_card = leaked_selector(format!("tasks-card-1-{}", pane_id.as_u64()));
    assert!(window.debug_bounds(task_card).is_none());
    assert!(inside(window, agent_card, "tasks-column-working"));
    assert_eq!(
        window.read(|app| view.read(app).main_view().tasks_selection.unwrap().pane_id),
        pane_id
    );
    assert!(window.debug_bounds("tasks-archive-task").is_some());
}

#[test]
fn a_task_started_again_waits_afresh_for_its_agent() {
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
    let task = create_task(&mut cli, "Fix the login redirect", None);
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 1));
    let _sent = capture(window, &view);
    let mut record = gui_tasks(window, &view)[&task].clone();
    let waiting = |window: &mut VisualTestContext| {
        window.read(|app| {
            view.read(app)
                .connection(1)
                .unwrap()
                .device_tasks
                .waiting
                .contains_key(&task)
        })
    };
    record.workspace = Some(workspace_id);
    event(window, &view, SessionEvent::TaskChanged(record.clone()));
    assert!(waiting(window));
    // Its Workspace closed: back in To do, and the wait is over.
    record.workspace = None;
    event(window, &view, SessionEvent::TaskChanged(record.clone()));
    assert!(!waiting(window));
    window.executor().advance_clock(Duration::from_secs(20));
    record.workspace = Some(workspace_id);
    event(window, &view, SessionEvent::TaskChanged(record));
    // The first Start's thirty seconds pass; the second's have not.
    window.executor().advance_clock(Duration::from_secs(15));
    window.run_until_parked();
    assert!(
        waiting(window),
        "the first Start's timer leaves the second's wait"
    );
    window.executor().advance_clock(Duration::from_secs(20));
    window.run_until_parked();
    assert!(!waiting(window));
}

#[test]
fn a_recovery_bootstrap_forgets_answers_it_dropped_and_asks_again() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, mut cli) = tasks_gui(&mut cx);
    let task = create_task(&mut cli, "Fix the login redirect", None);
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 1));
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    draw(window);
    open_form(window, &view, task);
    set_title(window, &view, "Fix the login redirect!");
    // A save whose answer, and an Agent request whose answer, writer lag dropped.
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            let connection = this.connection_mut(1).unwrap();
            connection.device_tasks.pending.insert(
                900,
                crate::app::tasks::TaskRequest::Edit {
                    title: "Fix the login redirect!".into(),
                },
            );
            connection
                .device_tasks
                .agent_requests
                .push_front(crate::app::tasks::AgentRequest::Hooks);
            connection.recover_after_bootstrap = true;
            connection.request_snapshot();
            this.task_form.as_mut().unwrap().saving = Some((1, 900));
        })
    });
    assert!(wait_until(window, |window| window.read(|app| {
        !view
            .read(app)
            .connection(1)
            .unwrap()
            .device_tasks
            .pending
            .contains_key(&900)
    })));
    assert!(window.read(|app| {
        let view = view.read(app);
        view.task_form.as_ref().unwrap().saving.is_none()
            && !view
                .connection(1)
                .unwrap()
                .device_tasks
                .agent_requests
                .contains(&crate::app::tasks::AgentRequest::Hooks)
    }));
    // Save works again, and the Agents are asked for again.
    click(window, "task-form-save");
    assert!(wait_until(window, |window| gui_tasks(window, &view)[&task]
        .title
        .ends_with('!')));
    assert!(wait_until(window, |window| window.read(|app| {
        view.read(app)
            .connection(1)
            .unwrap()
            .device_tasks
            .available
            .is_some()
    })));
}

#[test]
fn agent_answers_land_where_they_were_asked() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, _cli) = tasks_gui(&mut cx);
    let _sent = capture(window, &view);
    use crate::app::tasks::AgentRequest;
    let answer = |window: &mut VisualTestContext, request: AgentRequest| {
        window.update(|_, cx| {
            view.update(cx, |this, cx| {
                let connection = this.connection_mut(1).unwrap();
                connection.device_tasks.agent_requests.clear();
                connection.device_tasks.agent_requests.push_back(request);
                connection.hooks_error = None;
                this.last_error = None;
                cx.notify();
            })
        });
        incoming(
            window,
            &view,
            ServerMessage::AgentResult {
                result: Err(condr_core::protocol::AgentError {
                    code: "io".into(),
                    message: "permission denied".into(),
                }),
            },
        );
        window.read(|app| {
            let view = view.read(app);
            (
                view.last_error.is_some(),
                view.connection(1).unwrap().hooks_error.is_some(),
            )
        })
    };
    assert_eq!(
        answer(window, AgentRequest::Available),
        (true, false),
        "the Agents on PATH: a toast"
    );
    assert_eq!(
        answer(window, AgentRequest::Hooks),
        (false, true),
        "Settings' hooks: its page"
    );
    assert_eq!(
        answer(window, AgentRequest::InstallHooks),
        (true, false),
        "Start's Install: a toast"
    );
}
