//! The interaction area: selecting a card, the single-Pane Dock that shows its Agent,
//! the keyboard, and the split between the cards and the area (ADR 0068).

use super::*;
use gpui_kit::component::resizable::resize_handle;
use rust_i18n::t;

impl Condr {
    /// Selects a card and opens the interaction area on its Agent's Pane. With
    /// `keyboard`, its terminal takes the keyboard, and with it the PTY's sizing (ADR
    /// 0036); without, the area previews the Pane and the card area keeps the keyboard.
    pub(in crate::app) fn select_task_card(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // An open form with changes asks before it goes (ADR 0068).
        self.leave_task_form(
            Box::new(move |this, window, cx| {
                this.select_card(key, pane_id, None, keyboard, window, cx)
            }),
            window,
            cx,
        );
    }

    /// Selects an Agent's card, or with `task` a started Task's own card on `pane_id`.
    pub(in crate::app) fn select_card(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        task: Option<TaskId>,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.task_form = None;
        let Some(connection) = self.connection(key) else {
            return;
        };
        let column = pane_sidebar_status(&connection.model, pane_id)
            .filter(|_| task.is_none())
            .map(TasksColumn::of);
        let shown = connection
            .presented_session()
            .and_then(|session| session.workspace_for_pane(pane_id))
            .is_some_and(|workspace| {
                Self::tasks_scope_shows(
                    self.effective_tasks_scope(self.tasks_view().tasks.scope),
                    connection,
                    workspace,
                )
            });
        let view = self.tasks_view_mut();
        // A notification's Agent outside the scope widens it to All first.
        if !shown {
            view.tasks.scope = TasksScope::All;
        }
        let kept = view
            .tasks_selection
            .filter(|selected| {
                (selected.key, selected.pane_id) == (key, pane_id) && selected.task == task
            })
            .and_then(|selected| selected.column);
        view.tasks_selection = Some(TasksSelection {
            key,
            pane_id,
            task,
            column: kept.or(column),
            keyboard,
        });
        self.rebuild_window_dock(window, cx);
        if keyboard {
            self.focus_tasks_terminal(key, pane_id, window, cx);
        } else {
            self.tasks_view().tasks_focus.clone().focus(window, cx);
        }
        self.sync_terminal_focus(window, cx);
        self.schedule_state_save(cx);
        cx.notify();
    }

    pub(super) fn focus_tasks_terminal(
        &self,
        key: ConnectionKey,
        pane_id: PaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(focus) = self
            .panels
            .get(&(key, pane_id))
            .map(|panel| panel.read(cx).focus_handle.clone())
        else {
            return;
        };
        let owner = cx.weak_entity();
        // The Panel mounts with the next render; focus it then and report the focus.
        window.defer(cx, move |window, cx| {
            focus.focus(window, cx);
            let _ = owner.update(cx, |this, cx| this.sync_terminal_focus(window, cx));
        });
    }

    /// Closes the interaction area, which ends the selection and leaves no terminal
    /// with the keyboard; a form with changes asks first.
    pub(in crate::app) fn close_task_area(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_task_form(
            Box::new(|this, window, cx| this.close_task_area_now(window, cx)),
            window,
            cx,
        );
    }

    pub(super) fn close_task_area_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.task_form = None;
        let view = self.tasks_view_mut();
        view.tasks_selection = None;
        view.tasks_dock = None;
        self.terminal_input.target = None;
        self.tasks_view().tasks_focus.clone().focus(window, cx);
        self.sync_terminal_focus(window, cx);
        cx.notify();
    }

    /// A click on a card: the selected one closes the area, another opens it with the
    /// keyboard in its terminal. A started Task's own card is known by its Task.
    pub(super) fn click_task_card(
        &mut self,
        key: ConnectionKey,
        pane_id: PaneId,
        task: Option<TaskId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selected = self
            .tasks_view()
            .tasks_selection
            .is_some_and(|selected| match task {
                Some(task) => selected.key == key && selected.task == Some(task),
                None => (selected.key, selected.pane_id) == (key, pane_id),
            });
        if selected {
            self.close_task_area(window, cx);
        } else {
            self.leave_task_form(
                Box::new(move |this, window, cx| {
                    this.select_card(key, pane_id, task, true, window, cx)
                }),
                window,
                cx,
            );
        }
    }

    /// The Agent cards in the order `next-agent` and the arrows step through: column
    /// by column, each by urgency.
    pub(super) fn task_card_order(&self) -> Vec<(ConnectionKey, PaneId)> {
        let view = self.tasks_view();
        self.task_cards(
            self.effective_tasks_scope(view.tasks.scope),
            view.tasks_selection,
        )
        .into_iter()
        .flat_map(|(_, cards)| cards)
        .filter(|card| card.task.is_none())
        .map(|card| (card.key, card.pane_id))
        .collect()
    }

    /// Steps the selection through the cards, wrapping at both ends; from no selection,
    /// forward starts at the first card and back at the last.
    pub(in crate::app) fn step_task_card(
        &mut self,
        step: isize,
        keyboard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let order = self.task_card_order();
        if order.is_empty() {
            return;
        }
        let current = self.tasks_view().tasks_selection.and_then(|selected| {
            order
                .iter()
                .position(|card| *card == (selected.key, selected.pane_id))
        });
        let next = match current {
            Some(ix) => (ix as isize + step).rem_euclid(order.len() as isize) as usize,
            None if step > 0 => 0,
            None => order.len() - 1,
        };
        let (key, pane_id) = order[next];
        self.select_task_card(key, pane_id, keyboard, window, cx);
    }

    /// The card area's keys: the arrows move the selection and keep the keyboard here,
    /// so neither an arrow nor Enter reaches an Agent by accident; Enter gives the
    /// selected terminal the keyboard.
    pub(super) fn tasks_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let stroke = &event.keystroke;
        if stroke.modifiers.modified() {
            return;
        }
        match stroke.key.as_str() {
            "up" | "left" => self.step_task_card(-1, false, window, cx),
            "down" | "right" => self.step_task_card(1, false, window, cx),
            "enter" => {
                if let Some(selected) = self.tasks_view().tasks_selection {
                    let (key, pane_id, task) = (selected.key, selected.pane_id, selected.task);
                    self.select_card(key, pane_id, task, true, window, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    /// A press on the card area's background takes the keyboard back from the terminal.
    pub(super) fn focus_task_cards(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = self.tasks_view().tasks_focus.clone();
        if let Some(selected) = self.tasks_view_mut().tasks_selection.as_mut() {
            selected.keyboard = false;
        }
        self.terminal_input.target = None;
        focus.focus(window, cx);
        self.sync_terminal_focus(window, cx);
        cx.notify();
    }

    pub(super) fn open_selected_in_workspace(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(selected) = self.tasks_view().tasks_selection else {
            return;
        };
        if self.window_view(window).scope == WindowScope::Tasks {
            // The main window shows it, and the area, keeping its card, yields to it
            // (ADR 0069).
            self.in_window(self.main_window(), cx, move |this, window, cx| {
                this.leave_tasks_mode(window, cx);
                this.select_pane(selected.key, selected.pane_id, window, cx);
                window.activate_window();
            });
            return;
        }
        self.tasks_view_mut().tasks_selection = None;
        self.leave_tasks_mode(window, cx);
        self.select_pane(selected.key, selected.pane_id, window, cx);
    }

    /// The other window that draws the selected Pane's Tab right now, if one does.
    pub(super) fn tasks_pane_shown_elsewhere(
        &self,
        selected: TasksSelection,
    ) -> Option<(WindowScope, AnyWindowHandle)> {
        self.window_drawing_pane(selected.key, selected.pane_id)
    }

    /// The Dock part of `rebuild_window_dock` in Tasks mode: no Tab's surface, and a
    /// Dock of the selected Pane alone, unless another window draws it.
    pub(in crate::app) fn rebuild_tasks_dock(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input_window = self.last_active == window.window_handle();
        self.window_view_mut(window).dock_surface = None;
        // A Pane that closed takes the area with it.
        if let Some(selected) = self.tasks_view().tasks_selection
            && self
                .connection(selected.key)
                .and_then(ServerConnection::session)
                .and_then(|session| session.workspace_for_pane(selected.pane_id))
                .is_none()
        {
            let view = self.tasks_view_mut();
            view.tasks_selection = None;
            view.tasks_dock = None;
        }
        let selection = self.tasks_view().tasks_selection;
        if input_window {
            self.terminal_input.target = selection
                .filter(|selected| selected.keyboard)
                .map(|selected| (selected.key, selected.pane_id));
        }
        let Some(selected) = selection else {
            self.tasks_view_mut().tasks_dock = None;
            return;
        };
        if self.tasks_pane_shown_elsewhere(selected).is_some() {
            self.tasks_view_mut().tasks_dock = None;
            return;
        }
        let pane = (selected.key, selected.pane_id);
        if self
            .tasks_view()
            .tasks_dock
            .as_ref()
            .is_some_and(|dock| dock.pane == pane)
        {
            return;
        }
        let size = {
            let workspace = self.tasks_view().workspace_size;
            size(
                workspace.width.max(px(1.)),
                (workspace.height / 2.).max(px(1.)),
            )
        };
        let layout =
            self.build_dock_layout(selected.key, &PaneLayout::Pane(selected.pane_id), size, cx);
        let area = cx.new(|cx| {
            DockArea::new("condr-tasks-dock", None, window, cx)
                .with_renderer(Rc::new(CondrDockRenderer))
        });
        area.update(cx, |dock, cx| {
            dock.set_locked(true, window, cx);
            dock.set_center(layout, window, cx);
        });
        self.tasks_view_mut().tasks_dock = Some(TasksDock { area, pane });
    }

    /// The cards, and below the Board or beside the List the interaction area, with a
    /// handle between them.
    pub(in crate::app) fn render_tasks_body(
        &self,
        view: &WindowView,
        board: Entity<TasksBoard>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        v_flex()
            .size_full()
            .child(self.tasks_toolbar(view.tasks, view.scope != WindowScope::Tasks, cx))
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(self.render_tasks_split(view, board, cx)),
            )
            .into_any_element()
    }

    /// The cards, and with a card selected the interaction area beside or below them.
    pub(super) fn render_tasks_split(
        &self,
        view: &WindowView,
        board: Entity<TasksBoard>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cards = AnyView::from(board).cached(StyleRefinement::default().size_full());
        let area = match (&self.task_form, view.tasks_selection) {
            (Some(form), _) => self.render_task_form_area(form, cx),
            (None, Some(selected)) => self.render_tasks_area(view, selected, cx),
            (None, None) => return div().size_full().child(cards).into_any_element(),
        };
        let board_layout = view.tasks.layout == TasksLayout::Board;
        let split = if board_layout {
            view.tasks.split_board.unwrap_or(DEFAULT_SPLIT_BOARD)
        } else {
            view.tasks.split_list.unwrap_or(DEFAULT_SPLIT_LIST)
        };
        let fraction = relative(f32::from(split) / 1000.);
        // Kit's handle straddles the area's leading edge and draws the rule, with a band
        // wider than the line to grab.
        let handle = resize_handle(
            "tasks-split",
            if board_layout {
                Axis::Vertical
            } else {
                Axis::Horizontal
            },
        )
        .debug_selector(|| "tasks-split".into())
        .on_drag(DraggedTasksSplit, |_, _, _, cx| cx.new(|_| EmptyView));
        let container = if board_layout { v_flex() } else { h_flex() };
        container
            .size_full()
            .on_drag_move(cx.listener(
                move |this, event: &DragMoveEvent<DraggedTasksSplit>, _, cx| {
                    let bounds = event.bounds;
                    let position = event.event.position;
                    let fraction = if board_layout {
                        (position.y - bounds.top()) / bounds.size.height
                    } else {
                        (position.x - bounds.left()) / bounds.size.width
                    };
                    let split = Some((fraction.clamp(0.2, 0.8) * 1000.).round() as u16);
                    let prefs = &mut this.tasks_view_mut().tasks;
                    if board_layout {
                        prefs.split_board = split;
                    } else {
                        prefs.split_list = split;
                    }
                    this.schedule_state_save(cx);
                    cx.notify();
                },
            ))
            .child(
                div()
                    .flex_none()
                    .map(|this| {
                        if board_layout {
                            this.h(fraction).w_full()
                        } else {
                            this.w(fraction).h_full()
                        }
                    })
                    .child(cards),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .size_full()
                    .min_w_0()
                    .min_h_0()
                    .child(area)
                    .child(handle),
            )
            .into_any_element()
    }

    /// The interaction area: a header of its own over the selected Agent's Pane.
    pub(super) fn render_tasks_area(
        &self,
        view: &WindowView,
        selected: TasksSelection,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (key, pane) = (selected.key, selected.pane_id);
        let connection = self.connection(key);
        let agent = connection.and_then(|connection| connection.model.agents.get(&pane));
        let card = connection.and_then(|connection| {
            let workspace = connection.presented_session()?.workspace_for_pane(pane)?;
            Some(TaskCard {
                key,
                workspace_id: workspace.id(),
                pane_id: pane,
                status: pane_sidebar_status(&connection.model, pane)?,
                task: None,
            })
        });
        // The Task whose Workspace the Pane is in, for its title and Archive task.
        let held = connection.and_then(|connection| {
            let workspace = connection.presented_session()?.workspace_for_pane(pane)?;
            let task = connection.model.workspace_task(workspace.id())?;
            Some((task.id, task.title.clone(), workspace.id()))
        });
        let task_card = selected.task.and_then(|task| {
            self.task_card_facts(
                &TaskCard {
                    key,
                    workspace_id: held.as_ref()?.2,
                    pane_id: pane,
                    status: super::sidebar::agent_sidebar_status(AgentDisplayState::Idle),
                    task: Some(task),
                },
                task,
            )
        });
        let facts = card.as_ref().and_then(|card| self.card_facts(card));
        let icon = card
            .as_ref()
            .zip(facts.as_ref())
            .map(|(card, facts)| self.card_icon(card, facts.kind, cx));
        let theme = cx.theme();
        let title = match (&facts, agent, &held) {
            (Some(facts), Some(_), _) => facts.title.clone(),
            (_, _, Some((_, title, _))) => title.clone(),
            _ => connection
                .and_then(|connection| connection.model.terminal_titles.get(&pane).cloned())
                .unwrap_or_default(),
        };
        let place = match (&facts, &task_card) {
            (Some(facts), _) => Some({
                let mut place = vec![facts.workspace.clone()];
                place.extend(facts.branch.clone());
                place.push(facts.kind.label().to_owned());
                place.push(facts.device.clone());
                place.join(" · ")
            }),
            (None, Some((_, place, device, _))) => Some(format!("{place} · {device}")),
            (None, None) => None,
        };
        let absent = match &task_card {
            Some((_, _, _, state)) => state.clone(),
            None => t!("tasks.area.agent_exited").into_owned(),
        };
        let archive = held.map(|(task, _, _)| {
            let owner = cx.weak_entity();
            Button::new("tasks-archive-task")
                .debug_selector(|| "tasks-archive-task".into())
                .ghost()
                .xsmall()
                .label(t!("tasks.archive.button"))
                .on_click(move |_, window, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        this.confirm_archive_task(key, task, window, cx)
                    });
                })
        });
        let open_owner = cx.weak_entity();
        let close_owner = cx.weak_entity();
        let header = h_flex()
            .debug_selector(|| "tasks-area-header".into())
            .flex_none()
            .h(rems(2.25))
            .w_full()
            .gap_2()
            .px_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .children(icon)
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .font_semibold()
                    .child(title),
            )
            .children(place.map(|place| {
                div()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(place)
            }))
            .when(agent.is_none(), |this| {
                this.child(
                    div()
                        .debug_selector(|| "tasks-agent-exited".into())
                        .flex_none()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(absent),
                )
            })
            .child(div().flex_1())
            .children(archive)
            .child(
                Button::new("tasks-open-in-workspace")
                    .debug_selector(|| "tasks-open-in-workspace".into())
                    .ghost()
                    .xsmall()
                    .label(t!("tasks.area.open_in_workspace"))
                    .on_click(move |_, window, cx| {
                        let _ = open_owner
                            .update(cx, |this, cx| this.open_selected_in_workspace(window, cx));
                    }),
            )
            .child(
                Button::new("tasks-area-close")
                    .debug_selector(|| "tasks-area-close".into())
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::Close))
                    .tooltip(t!("tasks.area.close"))
                    .on_click(move |_, window, cx| {
                        let _ = close_owner.update(cx, |this, cx| this.close_task_area(window, cx));
                    }),
            );
        let body = match self.tasks_pane_shown_elsewhere(selected) {
            Some((scope, handle)) => self.render_shown_elsewhere(scope, handle, cx),
            None => match &view.tasks_dock {
                Some(dock) => div()
                    .debug_selector(|| "tasks-area-dock".into())
                    .size_full()
                    .child(dock.area.clone())
                    .into_any_element(),
                None => div().into_any_element(),
            },
        };
        // A Device retrying on its own veils its Pane, as its Workspace is in normal mode.
        let veil = connection.and_then(|connection| self.render_reconnect_veil(connection, cx));
        v_flex()
            .id("tasks-area")
            .debug_selector(|| "tasks-area".into())
            .size_full()
            .child(header)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(body)
                    .children(veil),
            )
            .into_any_element()
    }
}
