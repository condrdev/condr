//! The Tasks window (ADR 0069): Tasks in a window of its own beside the main window.

use super::*;
use crate::app::window_view::WindowScope;
use gpui_kit::AnyWindowHandle;

fn tasks_window(window: &mut VisualTestContext, view: &Entity<Condr>) -> Option<AnyWindowHandle> {
    window.read(|app| view.read(app).window_for(WindowScope::Tasks))
}

/// Opens the Tasks window from Tasks mode's title, as a person does.
fn open_tasks_window(window: &mut VisualTestContext, view: &Entity<Condr>) -> VisualTestContext {
    click(window, "tasks-open-in-new-window");
    window.run_until_parked();
    let handle = tasks_window(window, view).expect("Tasks opens in a window of its own");
    let mut tasks = VisualTestContext::from_window(handle, window);
    tasks.run_until_parked();
    draw(&mut tasks);
    draw(window);
    tasks
}

fn selected(window: &mut VisualTestContext, view: &Entity<Condr>) -> Option<PaneId> {
    window.read(|app| {
        view.read(app)
            .tasks_view()
            .tasks_selection
            .map(|selected| selected.pane_id)
    })
}

#[test]
fn tasks_opens_in_its_own_window_beside_the_workspace_and_comes_back() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, pane_id) = tasks_mode_with_an_agent(&mut cx);
    let before = presented(window, &view);
    click(
        window,
        leaked_selector(format!("tasks-card-1-{}", pane_id.as_u64())),
    );
    assert!(inside(window, terminal_selector(pane_id), "tasks-area"));

    let mut tasks = open_tasks_window(window, &view);
    // The main window is back on the Workspace it showed, and draws the Agent's Tab.
    assert!(window.debug_bounds("tasks-mode").is_none());
    assert_eq!(presented(window, &view), before);
    assert!(window.debug_bounds(terminal_selector(pane_id)).is_some());
    assert!(tasks.debug_bounds("tasks-mode").is_some());
    assert!(
        tasks.debug_bounds("condr-sidebar").is_none(),
        "the Tasks window has no sidebar"
    );
    assert!(tasks.debug_bounds("bring-back-window").is_some());
    assert!(tasks.debug_bounds("tasks-close").is_none());
    assert!(tasks.debug_bounds("tasks-open-in-new-window").is_none());
    assert_eq!(
        selected(window, &view),
        Some(pane_id),
        "the selection moved with Tasks"
    );
    // A terminal is never drawn twice: the area yields to the main window.
    assert!(inside(&mut tasks, "shown-elsewhere", "tasks-area"));
    assert!(tasks.debug_bounds(terminal_selector(pane_id)).is_none());

    // The Tasks window shows Tasks alone, whatever the sidebar and Tasks toggles say.
    tasks.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.toggle_sidebar(window, cx);
            this.toggle_tasks(window, cx);
        })
    });
    draw(&mut tasks);
    assert!(tasks.debug_bounds("tasks-mode").is_some());
    assert!(tasks.debug_bounds("condr-sidebar").is_none());

    // The main window's Tasks mode is now the page saying where Tasks is; drawing no Tab
    // any more, it leaves the terminal to the area.
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    window.run_until_parked();
    draw(window);
    tasks.run_until_parked();
    draw(&mut tasks);
    assert!(window.debug_bounds("shown-elsewhere").is_some());
    assert!(window.debug_bounds(terminal_selector(pane_id)).is_none());
    assert!(inside(&mut tasks, terminal_selector(pane_id), "tasks-area"));

    // Bring Back on that page: Tasks mode in the main window, the same card selected.
    click(window, "bring-back");
    window.run_until_parked();
    draw(window);
    assert!(tasks_window(window, &view).is_none());
    assert!(window.debug_bounds("tasks-mode").is_some());
    assert!(inside(window, terminal_selector(pane_id), "tasks-area"));
}

#[test]
fn closing_the_tasks_window_keeps_the_main_window_and_the_unsaved_form() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, _cli) = tasks_gui(&mut cx);
    window.update(|_, cx| {
        view.update(cx, |this, _| {
            this.send_layout(LayoutCommand::CreateWorkspace {
                name: None,
                root_directory: std::env::temp_dir(),
            });
        });
    });
    assert!(wait_until(window, |window| presented(window, &view).is_some()));
    let before = presented(window, &view);
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    draw(window);
    click(window, "tasks-new-task");
    window.simulate_input("Draft");
    let title = |window: &mut VisualTestContext| {
        window.read(|app| {
            view.read(app)
                .task_form
                .as_ref()
                .map(|form| form.title.read(app).value().to_string())
        })
    };

    let mut tasks = open_tasks_window(window, &view);
    assert!(
        tasks.debug_bounds("task-form").is_some(),
        "the unsaved form moved with Tasks"
    );
    // Its inputs were made again in the Tasks window, and take typing there.
    click(&mut tasks, "task-form-title");
    tasks.simulate_input(" more");
    assert_eq!(title(window).as_deref(), Some("Draft more"));

    // Closing the window puts Tasks away and asks nothing; the main window keeps showing
    // its Workspace.
    tasks.update(|window, cx| {
        view.update(cx, |this, cx| this.bring_back(window, cx));
        window.remove_window();
    });
    window.run_until_parked();
    draw(window);
    assert!(tasks_window(window, &view).is_none());
    assert!(window.debug_bounds("tasks-mode").is_none());
    assert_eq!(presented(window, &view), before);

    // Tasks mode then shows the same form, which takes typing in the main window again.
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    draw(window);
    assert!(window.debug_bounds("task-form").is_some());
    click(window, "task-form-title");
    window.simulate_input("!");
    assert_eq!(title(window).as_deref(), Some("Draft more!"));
}

#[test]
fn from_the_tasks_window_a_notification_and_open_in_workspace_go_where_the_agent_is_drawn() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, pane_id) = tasks_mode_with_an_agent(&mut cx);
    let mut tasks = open_tasks_window(window, &view);
    let main = window.update(|window, _| window.window_handle());
    let tasks_handle = tasks_window(window, &view).unwrap();
    let notify = |window: &mut VisualTestContext| {
        window.update(|window, cx| {
            view.update(cx, |this, cx| {
                this.handle_system_notification_response(
                    gpui_kit::SystemNotificationResponse {
                        tag: crate::app::notifications::pane_notification_tag(1, pane_id),
                        action_id: None,
                    },
                    window,
                    cx,
                )
            })
        });
        window.run_until_parked();
    };
    let active = |window: &mut VisualTestContext| window.read(|app| app.active_window());

    // The main window draws the Agent's Tab: the card is selected in the Tasks window, and
    // the main window, where the terminal is, comes forward.
    notify(window);
    assert_eq!(selected(window, &view), Some(pane_id));
    assert_eq!(active(window), Some(main));

    // With the main window on its Tasks page, the Tasks window comes forward instead.
    window.update(|window, cx| view.update(cx, |this, cx| this.toggle_tasks(window, cx)));
    window.run_until_parked();
    notify(window);
    assert_eq!(active(window), Some(tasks_handle));
    tasks.run_until_parked();
    draw(&mut tasks);
    assert!(inside(&mut tasks, terminal_selector(pane_id), "tasks-area"));

    // Open in Workspace shows the Pane in the main window, which comes forward; the card
    // stays selected and the area yields to it.
    click(&mut tasks, "tasks-open-in-workspace");
    window.run_until_parked();
    draw(window);
    draw(&mut tasks);
    assert_eq!(active(window), Some(main));
    assert!(window.debug_bounds("tasks-mode").is_none());
    assert!(window.debug_bounds(terminal_selector(pane_id)).is_some());
    assert_eq!(selected(window, &view), Some(pane_id));
    assert!(inside(&mut tasks, "shown-elsewhere", "tasks-area"));
}

#[test]
fn the_tasks_window_comes_back_after_a_restart() {
    let _serial_guard = acquire_visual_test_lock();
    let state_directory = TestDirectory::new("restart-tasks-window");
    let path = state_directory.0.join(gui_state::STATE_FILE_NAME);
    let (server, endpoint) = start_server();

    let mut first_cx = TestAppContext::single();
    first_cx.update(gpui_kit::init);
    let (first, first_window) = connected_condr_at(
        &mut first_cx,
        endpoint.clone(),
        gui_state::LoadedState::read(Some(path.clone())),
    );
    first_window.update(|window, cx| {
        first.update(cx, |this, cx| {
            this.toggle_tasks(window, cx);
            this.open_in_new_window(WindowScope::Tasks, window, cx);
        })
    });
    assert!(
        wait_until(first_window, |_| {
            gui_state::GuiState::read(Some(&path))
                .tasks_window
                .is_some()
        }),
        "the state file keeps the Tasks window"
    );
    drop(first_cx);

    let mut second_cx = TestAppContext::single();
    second_cx.update(gpui_kit::init);
    let (second, second_window) = connected_condr_at(
        &mut second_cx,
        endpoint,
        gui_state::LoadedState::read(Some(path.clone())),
    );
    assert!(
        wait_until(second_window, |window| tasks_window(window, &second)
            .is_some()),
        "the Tasks window opens again"
    );
    let handle = tasks_window(second_window, &second).unwrap();
    let mut tasks = VisualTestContext::from_window(handle, second_window);
    draw(&mut tasks);
    assert!(tasks.debug_bounds("tasks-mode").is_some());
    assert_eq!(
        second_window.read(|app| app.active_window()),
        Some(second_window.update(|window, _| window.window_handle())),
        "the main window is activated last"
    );
    drop(server);
}
