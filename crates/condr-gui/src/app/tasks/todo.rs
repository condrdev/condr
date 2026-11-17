//! To do's cards and rows, its Archived list and New task, and a started Task's card
//! while its Workspace has no Agent (ADR 0068).

use super::*;
use rust_i18n::t;

impl Condr {
    pub(super) fn render_new_task_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let owner = cx.weak_entity();
        Button::new("tasks-new-task")
            .debug_selector(|| "tasks-new-task".into())
            .ghost()
            .small()
            .w_full()
            .icon(Icon::new(IconName::Plus))
            .label(t!("tasks.todo.new"))
            .on_click(move |_, window, cx| {
                let _ = owner.update(cx, |this, cx| this.new_task(window, cx));
            })
            .into_any_element()
    }

    /// The Table's "+ New task": a row of its own, its mark and label where a Task's are.
    pub(super) fn render_new_task_row(&self, fit: ListFit, cx: &mut Context<Self>) -> AnyElement {
        let owner = cx.weak_entity();
        let theme = cx.theme();
        let hover = theme.list_hover;
        list_row(
            fit,
            "tasks-new-task",
            Icon::new(IconName::Plus)
                .size_3p5()
                .text_color(theme.muted_foreground),
            t!("tasks.todo.new").into_owned(),
            String::new(),
            String::new(),
            String::new(),
            div(),
            div(),
        )
        .text_sm()
        .text_color(theme.muted_foreground)
        .rounded(theme.radius)
        // The Task rows' selection border, so the label lines up with their titles.
        .border_1()
        .border_color(gpui_kit::transparent_black())
        .cursor_pointer()
        .hover(move |this| this.bg(hover))
        .on_click(move |_, window, cx| {
            let _ = owner.update(cx, |this, cx| this.new_task(window, cx));
        })
        .into_any_element()
    }

    /// `Device · repository` and `↺ branch` for a Task worked before.
    pub(super) fn todo_place(
        &self,
        key: ConnectionKey,
        task: &TaskRecord,
    ) -> (String, Option<String>) {
        let connection = self.connection(key);
        let mut place = vec![
            connection
                .map(|connection| connection.label.clone())
                .unwrap_or_default(),
        ];
        place.extend(
            task.repository
                .as_deref()
                .map(|root| repository_name(connection, root)),
        );
        let last = task.last.as_ref().map(|last| {
            let name = last.branch.clone().unwrap_or_else(|| {
                last.path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default()
            });
            format!("↺ {name}")
        });
        (place.join(" · "), last)
    }

    pub(super) fn todo_selected(&self, key: ConnectionKey, task: TaskId) -> bool {
        self.task_form
            .as_ref()
            .is_some_and(|form| form.home == Some((key, task)))
    }

    /// Makes a To do card draggable onto its Device's other To do cards.
    pub(super) fn todo_drag(
        &self,
        element: Stateful<Div>,
        key: ConnectionKey,
        task: &TaskRecord,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let (id, name) = (task.id, SharedString::from(task.title.clone()));
        let primary = cx.theme().primary;
        element
            .on_drag(DraggedTodo { key, task: id }, move |_, offset, _, cx| {
                cx.new(|_| super::sidebar::DragPreview {
                    icon: None,
                    name: name.clone(),
                    offset,
                })
            })
            .drag_over::<DraggedTodo>(move |style, drag, _, _| {
                if drag.key == key {
                    style.border_color(primary)
                } else {
                    style
                }
            })
            .on_drop(cx.listener(move |this, drag: &DraggedTodo, _, cx| {
                if drag.key == key {
                    this.reorder_todo(key, drag.task, id, cx);
                }
            }))
    }

    pub(super) fn render_todo_card(
        &self,
        key: ConnectionKey,
        task: &TaskRecord,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = task.id;
        let selected = self.todo_selected(key, id);
        let (place, last) = self.todo_place(key, task);
        let on_click = cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.click_todo_card(key, id, window, cx)
        });
        let theme = cx.theme().clone();
        let selector = format!("tasks-todo-{key}-{}", id);
        let starting = task.starting.then(|| {
            div()
                .debug_selector(move || format!("tasks-todo-starting-{key}-{}", id))
                .flex_none()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(t!("tasks.todo.starting").into_owned())
        });
        let mark = Icon::new(CondrIconName::Circle)
            .size_3p5()
            .text_color(theme.muted_foreground);
        let card = v_flex()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .w_full()
            .rounded(theme.radius)
            .border_1()
            .border_color(if selected {
                theme.primary
            } else if compact {
                gpui_kit::transparent_black()
            } else {
                theme.border
            })
            .cursor_pointer()
            .on_click(on_click);
        let card = if compact {
            card.px_1().py_0p5().child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .text_sm()
                    .child(mark)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(task.title.clone()),
                    )
                    .children(starting),
            )
        } else {
            card.gap_1()
                .p_2()
                .bg(theme.sidebar)
                .child(
                    h_flex()
                        .gap_2()
                        .items_start()
                        .child(mark)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .font_semibold()
                                .truncate()
                                .child(task.title.clone()),
                        )
                        .children(starting),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .min_w_0()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(div().min_w_0().truncate().child(place))
                        .children(last.map(|last| div().flex_none().child(last))),
                )
        };
        self.todo_drag(card, key, task, cx).into_any_element()
    }

    pub(super) fn render_todo_row(
        &self,
        key: ConnectionKey,
        task: &TaskRecord,
        fit: ListFit,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = task.id;
        let selected = self.todo_selected(key, id);
        let on_click = cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.click_todo_card(key, id, window, cx)
        });
        let theme = cx.theme().clone();
        let connection = self.connection(key);
        let repository = task
            .repository
            .as_deref()
            .map(|root| repository_name(connection, root))
            .unwrap_or_default();
        let (_, last) = self.todo_place(key, task);
        let state = div().text_color(theme.muted_foreground).children(
            task.starting
                .then(|| t!("tasks.todo.starting").into_owned()),
        );
        let row = list_row(
            fit,
            format!("tasks-todo-{key}-{}", id),
            Icon::new(CondrIconName::Circle)
                .size_3p5()
                .text_color(theme.muted_foreground),
            task.title.clone(),
            repository,
            last.unwrap_or_default(),
            connection
                .map(|connection| connection.label.clone())
                .unwrap_or_default(),
            div(),
            state,
        )
        .text_sm()
        .rounded(theme.radius)
        .border_1()
        .border_color(if selected {
            theme.primary
        } else {
            gpui_kit::transparent_black()
        })
        .cursor_pointer()
        .on_click(on_click);
        self.todo_drag(row, key, task, cx).into_any_element()
    }

    /// The folded Archived list at the bottom of To do, fetched when it unfolds, with
    /// Restore and Delete on each Task.
    pub(super) fn render_archived(&self, scope: TasksScope, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let open = self.tasks_archived_open;
        let owner = cx.weak_entity();
        let toggle = h_flex()
            .id("tasks-archived")
            .debug_selector(|| "tasks-archived".into())
            .gap_1()
            .px_1()
            .pt_2()
            .items_center()
            .text_xs()
            .text_color(theme.muted_foreground)
            .cursor_pointer()
            .child(
                Icon::new(IconName::ChevronRight)
                    .size_3()
                    .when(open, |icon| icon.rotate(percentage(90. / 360.))),
            )
            .child(t!("tasks.todo.archived").into_owned())
            .on_click(move |_, _, cx| {
                let _ = owner.update(cx, |this, cx| this.toggle_archived_tasks(cx));
            });
        let mut rows = Vec::new();
        if open {
            for connection in self.task_devices() {
                let in_scope = match scope {
                    TasksScope::All => true,
                    TasksScope::Device(server) | TasksScope::Workspace(server, _) => {
                        connection.model.server_id == Some(server)
                    }
                };
                let Some(archived) = connection
                    .device_tasks
                    .archived
                    .as_ref()
                    .filter(|_| in_scope)
                else {
                    continue;
                };
                for task in archived {
                    rows.push(self.render_archived_row(connection.key, task, &theme, cx));
                }
            }
        }
        v_flex()
            .w_full()
            .gap_0p5()
            .child(toggle)
            .children(rows)
            .into_any_element()
    }

    pub(super) fn render_archived_row(
        &self,
        key: ConnectionKey,
        task: &TaskRecord,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = task.id;
        let (restore_owner, delete_owner) = (cx.weak_entity(), cx.weak_entity());
        let title = task.title.clone();
        h_flex()
            .debug_selector(move || format!("tasks-archived-{key}-{}", id))
            .w_full()
            .gap_1()
            .pl(rems(1.))
            .items_center()
            .text_sm()
            .text_color(theme.muted_foreground)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(task.title.clone()),
            )
            .child(
                Button::new(SharedString::from(format!("tasks-restore-{key}-{}", id)))
                    .debug_selector(move || format!("tasks-restore-{key}-{}", id))
                    .ghost()
                    .xsmall()
                    .label(t!("tasks.todo.restore"))
                    .on_click(move |_, _, cx| {
                        let _ = restore_owner.update(cx, |this, cx| this.restore_task(key, id, cx));
                    }),
            )
            .child(
                Button::new(SharedString::from(format!(
                    "tasks-archived-delete-{key}-{}",
                    id
                )))
                .debug_selector(move || format!("tasks-archived-delete-{key}-{}", id))
                .ghost()
                .xsmall()
                .label(t!("tasks.form.delete"))
                .on_click(move |_, window, cx| {
                    let title = title.clone();
                    let _ = delete_owner.update(cx, |this, cx| {
                        this.confirm_delete_task(key, id, title, window, cx)
                    });
                }),
            )
            .into_any_element()
    }

    /// A started Task's title, `Workspace · branch` and Device, and what it waits for.
    pub(super) fn task_card_facts(
        &self,
        card: &TaskCard,
        task: TaskId,
    ) -> Option<(String, String, String, String)> {
        let connection = self.connection(card.key)?;
        let task = connection.model.tasks.as_ref()?.get(&task)?;
        let workspace = connection.session()?.workspace(card.workspace_id)?;
        let mut place = vec![workspace.name().to_owned()];
        place.extend(
            connection
                .model
                .workspace_git
                .get(&card.workspace_id)
                .and_then(|git| git.branch.clone()),
        );
        let state = if self.task_starting(card.key, task.id) {
            t!("tasks.todo.starting")
        } else {
            t!("tasks.todo.no_agent")
        };
        Some((
            task.title.clone(),
            place.join(" · "),
            connection.label.clone(),
            state.into_owned(),
        ))
    }

    pub(super) fn task_card_selected(&self, card: &TaskCard, task: TaskId) -> bool {
        self.main_view()
            .tasks_selection
            .is_some_and(|selected| selected.key == card.key && selected.task == Some(task))
    }

    pub(super) fn render_task_card(
        &self,
        card: &TaskCard,
        task: TaskId,
        compact: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some((title, place, device, state)) = self.task_card_facts(card, task) else {
            return div().into_any_element();
        };
        let selected = self.task_card_selected(card, task);
        let (key, pane_id) = (card.key, card.pane_id);
        let on_click = cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.click_task_card(key, pane_id, Some(task), window, cx)
        });
        let theme = cx.theme().clone();
        let selector = format!("tasks-task-{key}-{}", task);
        let mark = Icon::new(CondrIconName::Circle)
            .size_3p5()
            .text_color(card.status.tone.color(cx));
        let state = div()
            .debug_selector({
                let selector = selector.clone();
                move || format!("{selector}-state")
            })
            .flex_none()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(state);
        let element = v_flex()
            .id(SharedString::from(selector.clone()))
            .debug_selector(move || selector.clone())
            .w_full()
            .rounded(theme.radius)
            .border_1()
            .cursor_pointer()
            .on_click(on_click);
        if compact {
            element
                .px_1()
                .py_0p5()
                .border_color(if selected {
                    theme.primary
                } else {
                    gpui_kit::transparent_black()
                })
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .text_sm()
                        .child(mark)
                        .child(div().flex_1().min_w_0().truncate().child(title))
                        .child(state),
                )
                .into_any_element()
        } else {
            element
                .gap_1()
                .p_2()
                .bg(theme.sidebar)
                .border_color(if selected {
                    theme.primary
                } else {
                    theme.border
                })
                .child(
                    h_flex()
                        .gap_2()
                        .items_start()
                        .child(mark)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_sm()
                                .font_semibold()
                                .truncate()
                                .child(title),
                        )
                        .child(state),
                )
                .child(
                    h_flex()
                        .gap_1()
                        .min_w_0()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(div().min_w_0().truncate().child(place))
                        .child(device_tag(device, &theme)),
                )
                .into_any_element()
        }
    }

    pub(super) fn render_task_row(
        &self,
        card: &TaskCard,
        task: TaskId,
        fit: ListFit,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some((title, _, device, state)) = self.task_card_facts(card, task) else {
            return div().into_any_element();
        };
        let selected = self.task_card_selected(card, task);
        let (key, pane_id) = (card.key, card.pane_id);
        let on_click = cx.listener(move |this, _: &ClickEvent, window, cx| {
            this.click_task_card(key, pane_id, Some(task), window, cx)
        });
        let theme = cx.theme().clone();
        let connection = self.connection(key);
        let workspace = connection
            .and_then(|connection| connection.session()?.workspace(card.workspace_id))
            .map(|workspace| workspace.name().to_owned())
            .unwrap_or_default();
        let branch = connection
            .and_then(|connection| connection.model.workspace_git.get(&card.workspace_id))
            .and_then(|git| git.branch.clone())
            .unwrap_or_default();
        list_row(
            fit,
            format!("tasks-task-{key}-{}", task),
            Icon::new(CondrIconName::Circle)
                .size_3p5()
                .text_color(card.status.tone.color(cx)),
            title,
            workspace,
            branch,
            device,
            div(),
            div().text_color(theme.muted_foreground).child(state),
        )
        .text_sm()
        .rounded(theme.radius)
        .border_1()
        .border_color(if selected {
            theme.primary
        } else {
            gpui_kit::transparent_black()
        })
        .cursor_pointer()
        .on_click(on_click)
        .into_any_element()
    }
}
