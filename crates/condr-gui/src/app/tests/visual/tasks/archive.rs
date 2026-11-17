//! Archiving a started Task and what happens to its Workspace.

use super::*;

#[test]
fn archiving_a_started_task_closes_a_workspace_its_start_made_and_keeps_a_borrowed_one() {
    let _serial_guard = acquire_visual_test_lock();
    let directory = TestDirectory::new("tasks-archive");
    let mut session = Session::new();
    let repository = session.create_workspace(directory.0.clone()).unwrap();
    let made = session.create_workspace(directory.0.clone()).unwrap();
    let borrowed = session.create_workspace(directory.0.clone()).unwrap();
    let reopened = session.create_workspace(directory.0.clone()).unwrap();
    let snapshot_path = directory.0.join("session.snapshot");
    std::fs::write(&snapshot_path, session.snapshot().to_bytes().unwrap()).unwrap();
    let endpoint = Endpoint::local(directory.0.join("server.sock"));
    let server = start_server_with_config(
        ServerConfig::ephemeral(endpoint.as_local_path().unwrap())
            .with_snapshot_path(snapshot_path),
    );
    let mut cx = TestAppContext::single();
    cx.update(gpui_kit::init);
    let (view, window, _server) = connected_condr_with(&mut cx, server, endpoint);
    let pane = |window: &mut VisualTestContext, workspace_id: WorkspaceId| {
        window.read(|app| {
            let session = view.read(app).connection(1)?.session()?;
            Some(session.workspace(workspace_id)?.tabs()[0].panes()[0].id())
        })
    };
    assert!(wait_until(window, |window| pane(window, reopened).is_some()));
    let in_made = pane(window, made).unwrap();
    let sent = capture(window, &view);
    // The test Server's directory is no Git worktree, so the Client is told two are
    // Managed Worktrees of the repository.
    let snapshot = window.read(|app| {
        let mut session = view
            .read(app)
            .connection(1)
            .unwrap()
            .session()
            .unwrap()
            .clone();
        for worktree in [made, borrowed] {
            assert!(session.associate_worktree(
                worktree,
                repository,
                directory.0.clone(),
                true,
                None
            ));
        }
        session.snapshot()
    });
    event(
        window,
        &view,
        SessionEvent::LayoutChanged {
            snapshot,
            zoomed_panes: Vec::new(),
        },
    );
    // What the Server recorded at each Start's commit: a new worktree and a plain folder
    // opened again are the Start's own; an open Workspace, Managed Worktree or not, is
    // borrowed.
    let task = |id: u128, workspace_id, made_workspace| condr_core::Task {
        id: TaskId::from_u128(id),
        title: format!("task {id}"),
        description: None,
        repository: Some(directory.0.clone()),
        order: 0,
        archived: false,
        workspace: Some(workspace_id),
        made_workspace,
        starting: false,
        last: Some(condr_core::LastPlace {
            path: directory.0.clone(),
            branch: Some("worktree/fix".into()),
            base_branch: None,
            managed: true,
            repository: Some(directory.0.clone()),
        }),
    };
    put_task(window, &view, task(1, made, true));
    put_task(window, &view, task(2, borrowed, false));
    put_task(window, &view, task(3, reopened, true));
    put_task(window, &view, task(4, repository, false));
    agent_changed(
        window,
        &view,
        in_made,
        Some(AgentSnapshot {
            session_id: None,
            kind: AgentKind::Codex,
            state: AgentState::Blocked,
            detail: Some("Bash: cargo test".into()),
        }),
    );
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    draw(window);

    // The Agent carries its Task's title; the others, with no Agent, are cards of their
    // own.
    let facts = window.read(|app| {
        view.read(app)
            .archive_facts(1, TaskId::from_u128(1))
            .unwrap()
    });
    assert!(facts.created && facts.managed);
    assert_eq!(facts.agents, 1);
    assert_eq!(
        facts.busy,
        Some((AgentKind::Codex, condr_core::AgentDisplayState::Blocked))
    );
    let archived = |sent: &std::sync::mpsc::Receiver<ClientMessage>| {
        sent.try_iter().find_map(|message| match message {
            ClientMessage::Task {
                command:
                    TaskCommand::Archive {
                        task,
                        close_workspace,
                        remove_worktree,
                    },
                ..
            } => Some((task, close_workspace, remove_worktree)),
            _ => None,
        })
    };
    // Archives a Task from its card's area with the dialog's defaults; says whether the
    // dialog offered to remove the worktree.
    let archive = |window: &mut VisualTestContext, card: &'static str| {
        click(window, card);
        click(window, "tasks-archive-task");
        assert!(window.update(|window, cx| window.has_active_dialog(cx)));
        draw(window);
        let removable = window.debug_bounds("task-archive-remove").is_some();
        window.update(|window, cx| {
            window.dispatch_action(Box::new(Confirm { secondary: false }), cx)
        });
        window.run_until_parked();
        draw(window);
        removable
    };
    for (card, removable, expected, why) in [
        (
            leaked_selector(format!("tasks-card-1-{}", in_made.as_u64())),
            true,
            (TaskId::from_u128(1), true, false),
            "a new worktree its Start made closes by default; the worktree stays",
        ),
        (
            leaked_selector(format!("tasks-task-1-{}", TaskId::from_u128(2))),
            true,
            (TaskId::from_u128(2), false, false),
            "a borrowed Managed Worktree stays open by default",
        ),
        (
            leaked_selector(format!("tasks-task-1-{}", TaskId::from_u128(3))),
            false,
            (TaskId::from_u128(3), true, false),
            "a plain folder its Start opened again closes by default",
        ),
        (
            leaked_selector(format!("tasks-task-1-{}", TaskId::from_u128(4))),
            false,
            (TaskId::from_u128(4), false, false),
            "the repository's own Workspace, borrowed, stays open",
        ),
    ] {
        assert_eq!(
            archive(window, card),
            removable,
            "only a Managed Worktree can be removed: {why}"
        );
        assert_eq!(archived(&sent), Some(expected), "{why}");
    }
}
