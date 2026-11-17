//! To do: its order, its count, and a Device that keeps no Tasks.

use super::*;

#[test]
fn to_do_heads_tasks_mode_newest_first_with_its_count_and_drags_to_reorder() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, mut cli) = tasks_gui(&mut cx);
    let first = create_task(&mut cli, "Fix the login redirect", None);
    let second = create_task(&mut cli, "Write the release notes", None);
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 2));
    draw(window);
    assert!(
        window.debug_bounds("sidebar-tasks-count").is_some(),
        "the sidebar's row counts To do"
    );
    assert_eq!(window.read(|app| view.read(app).todo_count()), 2);

    // The Command Palette's New Task enters Tasks mode with the form; an empty one's
    // Cancel asks nothing.
    window.update(|window, cx| {
        // From the window's focus, as the palette runs it.
        let focus = view.read(cx).main_view().focus_handle.clone();
        focus.focus(window, cx);
        window.dispatch_action(Box::new(crate::app::NewTask), cx)
    });
    window.run_until_parked();
    draw(window);
    assert!(window.debug_bounds("tasks-mode").is_some());
    assert!(window.debug_bounds("task-form").is_some());
    click(window, "task-form-cancel");
    assert!(!window.update(|window, cx| window.has_active_dialog(cx)));
    assert!(window.debug_bounds("task-form").is_none());
    assert!(inside(window, todo_card(first), "tasks-column-to-do"));
    assert!(inside(window, todo_card(second), "tasks-column-to-do"));
    let (top, below) = (
        window.debug_bounds(todo_card(second)).unwrap(),
        window.debug_bounds(todo_card(first)).unwrap(),
    );
    assert!(top.top() < below.top(), "a new Task heads To do");
    let to_do = window.debug_bounds("tasks-column-to-do").unwrap();
    let needs_you = window.debug_bounds("tasks-column-needs-you").unwrap();
    assert!(to_do.left() < needs_you.left(), "To do is the first column");

    // Dragging the lower card onto the upper one moves it up, on the Server.
    window.simulate_mouse_down(below.center(), MouseButton::Left, Modifiers::default());
    for _ in 0..2 {
        window.simulate_mouse_move(top.center(), MouseButton::Left, Modifiers::default());
        window.run_until_parked();
        draw(window);
    }
    window.simulate_mouse_up(top.center(), MouseButton::Left, Modifiers::default());
    assert!(wait_until(window, |window| {
        let tasks = gui_tasks(window, &view);
        tasks[&first].order < tasks[&second].order
    }));

    // The List carries To do as its first group.
    click(window, "tasks-layout");
    click(window, "tasks-layout-list");
    assert!(window.debug_bounds(todo_card(first)).is_some());
    assert!(window.debug_bounds("tasks-new-task").is_some());
}

#[test]
fn a_device_whose_server_keeps_no_tasks_offers_no_to_do() {
    let _serial_guard = acquire_visual_test_lock();
    let mut cx = TestAppContext::single();
    let (view, window, _server, mut cli) = tasks_gui(&mut cx);
    create_task(&mut cli, "Fix the login redirect", None);
    assert!(wait_until(window, |window| gui_tasks(window, &view).len() == 1));
    // An older Server, or one built without Tasks, sends no Tasks record.
    window.update(|window, cx| {
        view.update(cx, |this, cx| {
            this.connection_mut(1).unwrap().model.tasks = None;
            this.toggle_tasks(window, cx);
        })
    });
    draw(window);
    assert!(window.debug_bounds("tasks-mode").is_some());
    assert!(window.debug_bounds("tasks-column-to-do").is_none());
    assert!(window.debug_bounds("tasks-column-needs-you").is_some());
    assert_eq!(window.read(|app| view.read(app).todo_count()), 0);
    window.update(|window, cx| view.update(cx, |this, cx| this.new_task(window, cx)));
    window.run_until_parked();
    assert!(
        window.read(|app| view.read(app).task_form.is_none()),
        "no Device takes a new Task"
    );
    assert!(window.read(|app| view.read(app).last_error.is_some()));
}
