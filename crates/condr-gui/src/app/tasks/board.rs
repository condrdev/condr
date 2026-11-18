//! The card area, a view of its own that a terminal frame never renders again, and the
//! Board: its columns, their headings and the strip a folded one becomes (ADR 0068).

use super::*;
use gpui_kit::component::scroll::ScrollableElement as _;
use rust_i18n::t;

pub(super) fn nothing_here(theme: &Theme) -> Div {
    div()
        .px_1()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(t!("tasks.nothing_here").into_owned())
}

/// A column's ring, in the colour its cards' state shows in.
pub(super) fn column_ring(column: TasksColumn, cx: &App) -> Icon {
    Icon::new(CondrIconName::Circle)
        .size_3p5()
        .text_color(column.tone().color(cx))
}

/// A count in a pill, beside a column's name.
pub(super) fn count_pill(count: usize, theme: &Theme) -> Div {
    div()
        .px_1p5()
        .rounded_full()
        .bg(theme.secondary)
        .text_xs()
        .font_normal()
        .text_color(theme.muted_foreground)
        .child(count.to_string())
}

/// A folded column's name turned a quarter clockwise, to read down its strip. GPUI does
/// not rotate text, so the name is SVG text, laid out in rems (16 units each) so it
/// follows the interface font. Its length is estimated, wide characters a full em.
pub(super) fn strip_label(label: &str, family: &str) -> Svg {
    const REM: f32 = 16.;
    // text_xs, and a baseline that centres its glyphs across the strip.
    let (size, baseline) = (0.75 * REM, 5.);
    let length = label
        .chars()
        .map(|letter| if letter.is_ascii() { 0.62 } else { 1. })
        .sum::<f32>()
        * size
        + 4.;
    let escape = |text: &str| {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('\'', "&apos;")
    };
    let data = format!(
        "<svg xmlns='http://www.w3.org/2000/svg' width='{REM}' height='{length}' \
         viewBox='0 0 {REM} {length}'><text transform='translate({baseline} 2) rotate(90)' \
         font-family='\"{}\", \"Segoe UI\", \"Helvetica Neue\", Helvetica, Arial, \
         \"Noto Sans\", \"DejaVu Sans\", sans-serif' font-size='{size}' font-weight='600'>{}\
         </text></svg>",
        // Quoted: a family such as `.SystemUIFont` is no CSS identifier, and an
        // unparsable list falls back to a serif.
        escape(&family.replace('"', "")),
        escape(label),
    );
    svg()
        .data(data.as_bytes())
        .flex_none()
        .w(rems(1.))
        .h(rems(length / REM))
}

/// The card area as a view of its own, cached as the sidebar is: a terminal frame
/// re-renders `Condr` and reuses the cards, which render again when `Condr` notifies.
pub(in crate::app) struct TasksBoard {
    owner: WeakEntity<Condr>,
    _owner_changed: Subscription,
}

impl TasksBoard {
    pub(in crate::app) fn new(owner: &Entity<Condr>, cx: &mut Context<Self>) -> Self {
        Self {
            owner: owner.downgrade(),
            _owner_changed: cx.observe(owner, |_, _, cx| cx.notify()),
        }
    }
}

impl Render for TasksBoard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.owner
            .update(cx, |condr, cx| {
                if !condr.owns_window(window) {
                    return div().into_any_element();
                }
                #[cfg(feature = "test-support")]
                condr
                    .tasks_board_render_count
                    .set(condr.tasks_board_render_count.get() + 1);
                let condr = &*condr;
                condr.render_task_cards(condr.window_view(window), window, cx)
            })
            .unwrap_or_else(|_| div().into_any_element())
    }
}

impl Condr {
    /// The card area, which `TasksBoard` caches (ADR 0068).
    pub(in crate::app) fn render_task_cards(
        &self,
        view: &WindowView,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let scope = self.effective_tasks_scope(view.tasks.scope);
        let columns = self.task_cards(scope, view.tasks_selection);
        let todo = self.offers_todo(scope).then(|| self.todo_cards(scope));
        let cards = match view.tasks.layout {
            TasksLayout::Board => self.render_tasks_board(
                &columns,
                todo,
                scope,
                view.tasks,
                &view.tasks_board_scroll,
                cx,
            ),
            TasksLayout::List => {
                let fit = ListFit::of(view, self.task_form.is_some(), window);
                self.render_tasks_list(&columns, todo, scope, view.tasks, fit, cx)
            }
        };
        v_flex()
            .debug_selector(|| "tasks-cards".into())
            .size_full()
            .track_focus(&view.tasks_focus)
            .on_key_down(cx.listener(Self::tasks_key_down))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.focus_task_cards(window, cx)),
            )
            .child(cards)
            .into_any_element()
    }

    pub(super) fn render_tasks_board(
        &self,
        columns: &[(TasksColumn, Vec<TaskCard>)],
        todo: Option<Vec<(ConnectionKey, TaskRecord)>>,
        scope: TasksScope,
        prefs: TasksPrefs,
        scroll: &ScrollHandle,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let density = prefs.density;
        let theme = cx.theme().clone();
        let todo = todo.map(|todo| {
            if prefs.is_folded(TasksColumn::ToDo) {
                return self.render_strip(TasksColumn::ToDo, todo.len(), cx);
            }
            v_flex()
                .id("to-do")
                .debug_selector(|| "tasks-column-to-do".into())
                .flex_1()
                .min_w(rems(14.))
                .h_full()
                .gap_2()
                .child(self.board_heading(TasksColumn::ToDo, todo.len(), &theme, cx))
                .child(
                    v_flex()
                        .id("tasks-column-cards-to-do")
                        .flex_1()
                        .min_h_0()
                        .gap_2()
                        .overflow_y_scrollbar()
                        .child(self.render_new_task_button(cx))
                        .children(todo.iter().map(|(key, task)| {
                            self.render_todo_card(*key, task, density == TasksDensity::Compact, cx)
                        }))
                        .child(self.render_archived(scope, cx)),
                )
                .into_any_element()
        });
        // GPUI's own scrolling, which turns a mouse's vertical wheel into the Board's
        // horizontal scroll, under Kit's scrollbar; Kit's scroll area would lock the axis.
        let board = h_flex()
            .id("tasks-board")
            .debug_selector(|| "tasks-board".into())
            .size_full()
            .items_start()
            .gap_3()
            .p_3()
            .overflow_x_scroll()
            .track_scroll(scroll)
            .children(todo)
            .children(columns.iter().map(|(column, cards)| {
                let column = *column;
                if prefs.is_folded(column) {
                    return self.render_strip(column, cards.len(), cx);
                }
                v_flex()
                    .id(column.selector())
                    .debug_selector(move || format!("tasks-column-{}", column.selector()))
                    .flex_1()
                    .min_w(rems(14.))
                    .h_full()
                    .gap_2()
                    .child(self.board_heading(column, cards.len(), &theme, cx))
                    .child(
                        v_flex()
                            .id(SharedString::from(format!(
                                "tasks-column-cards-{}",
                                column.selector()
                            )))
                            .flex_1()
                            .min_h_0()
                            .gap_2()
                            .overflow_y_scrollbar()
                            // Every column from this one call site: the scroll state is
                            // its own only under an id of its own.
                            .id(SharedString::from(format!(
                                "tasks-column-scroll-{}",
                                column.selector()
                            )))
                            .map(|this| {
                                if cards.is_empty() {
                                    this.child(nothing_here(&theme))
                                } else {
                                    this.children(cards.iter().map(|card| {
                                        match (card.task, density) {
                                            (Some(task), _) => self.render_task_card(
                                                card,
                                                task,
                                                density == TasksDensity::Compact,
                                                cx,
                                            ),
                                            (None, TasksDensity::Cards) => {
                                                self.render_card(card, cx)
                                            }
                                            (None, TasksDensity::Compact) => {
                                                self.render_compact_card(card, cx)
                                            }
                                        }
                                    }))
                                }
                            }),
                    )
                    .into_any_element()
            }));
        div()
            .relative()
            .size_full()
            .child(board)
            .horizontal_scrollbar(scroll)
            .into_any_element()
    }

    /// A folded column: its count over its name, turned to read down the strip; a click
    /// unfolds it.
    pub(super) fn render_strip(
        &self,
        column: TasksColumn,
        count: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let owner = cx.weak_entity();
        let theme = cx.theme();
        v_flex()
            .id(SharedString::from(format!(
                "tasks-strip-{}",
                column.selector()
            )))
            .debug_selector(move || format!("tasks-strip-{}", column.selector()))
            .flex_none()
            .w(rems(2.25))
            .h_full()
            .items_center()
            .gap_1()
            .py_2()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .text_xs()
            .text_color(theme.muted_foreground)
            .cursor_pointer()
            .child(column_ring(column, cx))
            .child(div().font_semibold().child(count.to_string()))
            .child(
                strip_label(&column.label(), &theme.font_family).text_color(theme.muted_foreground),
            )
            .on_click(move |_, _, cx| {
                let _ = owner.update(cx, |this, cx| this.toggle_tasks_column(column, cx));
            })
            .into_any_element()
    }

    /// A column's or group's ring, name and count; a click folds it.
    pub(super) fn column_heading(
        &self,
        column: TasksColumn,
        count: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let owner = cx.weak_entity();
        h_flex()
            .id(SharedString::from(format!(
                "tasks-heading-{}",
                column.selector()
            )))
            .debug_selector(move || format!("tasks-heading-{}", column.selector()))
            .gap_2()
            .px_1()
            .py_0p5()
            .rounded(theme.radius)
            .text_sm()
            .font_semibold()
            .text_color(theme.foreground)
            .cursor_pointer()
            .hover(|this| this.bg(theme.list_hover))
            .child(column_ring(column, cx))
            .child(column.label())
            .child(count_pill(count, theme))
            .on_click(move |_, _, cx| {
                let _ = owner.update(cx, |this, cx| this.toggle_tasks_column(column, cx));
            })
    }

    /// A Board column's heading, across the column with the fold mark at its end.
    pub(super) fn board_heading(
        &self,
        column: TasksColumn,
        count: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        self.column_heading(column, count, theme, cx)
            .w_full()
            .child(div().flex_1())
            .child(
                Icon::new(gpui_kit::assets::IconName::FoldHorizontal)
                    .xsmall()
                    .text_color(theme.muted_foreground),
            )
    }
}
