//! Tasks mode and Tasks (ADR 0068): the fixtures and helpers its tests share; the
//! tests themselves are in a submodule per part.

use super::layout::agent_changed;
use super::*;
use condr_core::{StartPlace, TaskCommand, TaskId, TaskResponse};

mod archive;
mod area;
mod form;
mod mode;
mod start;
mod todo;
mod window;

const SPLIT_RIGHT: &str = if cfg!(target_os = "macos") {
    "cmd-d"
} else {
    "alt-shift-="
};

fn presented(window: &mut VisualTestContext, view: &Entity<Condr>) -> Option<(WorkspaceId, TabId)> {
    window.read(|app| {
        view.read(app)
            .presented()
            .map(|(_, _, workspace_id, tab_id)| (workspace_id, tab_id))
    })
}

fn draw(window: &mut VisualTestContext) {
    window.update(|window, cx| _ = window.draw(cx));
}

fn inside(window: &mut VisualTestContext, inner: &'static str, outer: &'static str) -> bool {
    let (Some(inner), Some(outer)) = (window.debug_bounds(inner), window.debug_bounds(outer))
    else {
        return false;
    };
    outer.contains(&inner.center())
}

const NEXT_AGENT: &str = if cfg!(target_os = "macos") {
    "cmd-]"
} else {
    "alt-]"
};

/// A Workspace with one Pane whose Agent works, in Tasks mode.
fn tasks_mode_with_an_agent(
    cx: &mut TestAppContext,
) -> (Entity<Condr>, &mut VisualTestContext, TestServer, PaneId) {
    tasks_mode_with_workspaces(cx, 1)
}

/// `count` Workspaces, the first's Pane running an Agent that works, in Tasks mode. The
/// Workspaces come first: a hand-made Agent event moves the Client's sequence past the
/// Server's next one.
fn tasks_mode_with_workspaces(
    cx: &mut TestAppContext,
    count: usize,
) -> (Entity<Condr>, &mut VisualTestContext, TestServer, PaneId) {
    cx.update(gpui_kit::init);
    let (view, window, server) = connected_condr(cx);
    for index in 0..count {
        // One directory each: a second Workspace on the same root waits for the first.
        let root_directory = std::env::temp_dir().join(format!("condr-tasks-{index}"));
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
    let mut pane_id = None;
    assert!(wait_until(window, |window| {
        pane_id = window.read(|app| {
            let workspaces = view.read(app).active_session()?.workspaces();
            if workspaces.len() < count {
                return None;
            }
            workspaces
                .first()?
                .tabs()
                .first()?
                .focused_pane()
                .map(|pane| pane.id())
        });
        pane_id.is_some_and(|pane_id| {
            // The first Workspace's terminal has its view, drawn or not.
            window.read(|app| view.read(app).terminal(1, pane_id).is_some())
        })
    }));
    let pane_id = pane_id.unwrap();
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
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    draw(window);
    (view, window, server, pane_id)
}

fn target(window: &mut VisualTestContext, view: &Entity<Condr>) -> Option<(u64, PaneId)> {
    window.read(|app| view.read(app).terminal_input.target)
}

/// A GUI on a fresh Server that keeps its Tasks in memory, and another Client of it, as
/// the CLI would be.
fn tasks_gui(
    cx: &mut TestAppContext,
) -> (
    Entity<Condr>,
    &mut VisualTestContext,
    TestServer,
    ClientConnection,
) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        super::super::super::startup::bind_keys(cx);
    });
    let (server, endpoint) = start_server();
    let cli = ClientConnection::connect_overview(&endpoint, "cli").unwrap();
    let (view, window, server) = connected_condr_with(cx, server, endpoint);
    (view, window, server, cli)
}

fn create_task(
    cli: &mut ClientConnection,
    title: &str,
    repository: Option<std::path::PathBuf>,
) -> TaskId {
    let Ok(Ok(TaskResponse::Task(task))) = cli.task(TaskCommand::Create {
        title: title.into(),
        description: format!("{title}, described"),
        repository,
    }) else {
        panic!("the Server keeps Tasks");
    };
    task.id
}

fn gui_tasks(
    window: &mut VisualTestContext,
    view: &Entity<Condr>,
) -> std::collections::BTreeMap<TaskId, condr_core::Task> {
    window.read(|app| {
        view.read(app)
            .connection(1)
            .and_then(|connection| connection.model.tasks.clone())
            .unwrap_or_default()
    })
}

fn click(window: &mut VisualTestContext, selector: &'static str) {
    let bounds = window
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    window.simulate_click(bounds.center(), Modifiers::default());
    window.run_until_parked();
    draw(window);
}

fn todo_card(task: TaskId) -> &'static str {
    leaked_selector(format!("tasks-todo-1-{}", task))
}

fn form_home(window: &mut VisualTestContext, view: &Entity<Condr>) -> Option<(u64, TaskId)> {
    window.read(|app| view.read(app).task_form.as_ref().and_then(|form| form.home))
}

/// A message from the Server, made by hand.
fn incoming(window: &mut VisualTestContext, view: &Entity<Condr>, message: ServerMessage) {
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            let generation = this.connection(1).unwrap().connect_generation;
            this.handle_incoming(1, generation, Incoming::Message(message), cx);
            cx.notify();
        });
    });
    window.run_until_parked();
    draw(window);
}

/// A Session event made by hand, after the Client's last.
fn event(window: &mut VisualTestContext, view: &Entity<Condr>, event: SessionEvent) {
    let message = window.read(|app| {
        let connection = view.read(app).connection(1).unwrap();
        ServerMessage::Event {
            server_id: connection.model.server_id.unwrap(),
            session_id: connection.model.session_id.unwrap(),
            sequence: connection.model.sequence + 1,
            event,
        }
    });
    incoming(window, view, message);
}

/// What the GUI sends from here on, which then never reaches the Server.
fn capture(
    window: &mut VisualTestContext,
    view: &Entity<Condr>,
) -> std::sync::mpsc::Receiver<ClientMessage> {
    let (outgoing, sent) = std::sync::mpsc::channel();
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.connection_mut(1).unwrap().io = Some(ClientIo {
                outgoing,
                _incoming_task: Task::ready(()),
            });
        });
    });
    sent
}

/// Puts a Task in the GUI's model as an event would, without one.
fn put_task(window: &mut VisualTestContext, view: &Entity<Condr>, task: condr_core::Task) {
    window.update(|_, cx| {
        view.update(cx, |this, cx| {
            this.connection_mut(1).unwrap().model.apply_task(task);
            cx.notify();
        });
    });
    draw(window);
}

/// A Task on the test's own second Device.
fn moved_task(id: u128) -> condr_core::Task {
    condr_core::Task {
        id: TaskId::from_u128(id),
        title: format!("Moved {id}"),
        description: Some("Fix the login redirect, described".into()),
        repository: None,
        order: 0,
        archived: false,
        workspace: None,
        made_workspace: false,
        starting: false,
        last: None,
    }
}

/// Opens a To do Task's form and waits for its description.
fn open_form(window: &mut VisualTestContext, view: &Entity<Condr>, task: TaskId) {
    click(window, todo_card(task));
    assert!(wait_until(window, |window| window.read(|app| {
        view.read(app)
            .task_form
            .as_ref()
            .is_some_and(|form| form.description_loaded())
    })));
    draw(window);
}

fn form_text(window: &mut VisualTestContext, view: &Entity<Condr>) -> (String, String) {
    window.read(|app| {
        let form = view.read(app).task_form.as_ref().unwrap();
        (
            form.title.read(app).value().to_string(),
            form.description.read(app).value().to_string(),
        )
    })
}

fn set_title(window: &mut VisualTestContext, view: &Entity<Condr>, title: &str) {
    let input = window.read(|app| view.read(app).task_form.as_ref().unwrap().title.clone());
    let title = title.to_owned();
    window.update(|window, cx| input.update(cx, |input, cx| input.set_value(title, window, cx)));
    draw(window);
}

fn form_error(
    window: &mut VisualTestContext,
    view: &Entity<Condr>,
) -> Option<super::super::super::tasks::FormField> {
    window.read(|app| {
        view.read(app)
            .task_form
            .as_ref()
            .and_then(|form| form.error.as_ref())
            .map(|(field, _)| *field)
    })
}
