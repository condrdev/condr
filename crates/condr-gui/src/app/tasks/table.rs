//! The Table: one compact row per card under each column's heading, its optional
//! columns dropped as it narrows (ADR 0068).

use super::*;
use gpui_kit::component::scroll::ScrollableElement as _;
use rust_i18n::t;

/// How many of the List's optional columns fit its width, taken in the order
/// Workspace, Device, Changes, Branch; the mark, Title and State always show, so a List
/// beside the interaction area keeps its titles.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::app) struct ListFit(pub(super) usize);

impl ListFit {
    /// Rems of the optional columns, each with its gap, in the order they are kept.
    const OPTIONAL: [f32; 4] = [9.5, 6.5, 6.5, 9.5];
    /// Rems of what always shows: padding, the mark, a Title's least, State and gaps.
    pub(super) const ALWAYS: f32 = 2. + 1.25 + 10. + 12. + 1.;

    pub(super) fn of(view: &WindowView, form_open: bool, window: &Window) -> Self {
        // The card area spans the window beside the left sidebar (the Changes sidebar is
        // under it); beside the interaction area the List takes its share of the split.
        let mut width =
            (window.viewport_size().width - view.sidebar_layout_width()) / window.rem_size();
        if form_open || view.tasks_selection.is_some() {
            width *= f32::from(view.tasks.split_list.unwrap_or(DEFAULT_SPLIT_LIST)) / 1000.;
        }
        Self::within(width)
    }

    pub(super) fn within(rems: f32) -> Self {
        let mut used = Self::ALWAYS;
        Self(
            Self::OPTIONAL
                .iter()
                .take_while(|column| {
                    used += *column;
                    used <= rems
                })
                .count(),
        )
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn list_row(
    fit: ListFit,
    id: impl Into<SharedString>,
    icon: impl IntoElement,
    title: String,
    workspace: String,
    branch: impl IntoElement,
    device: String,
    changes: impl IntoElement,
    state: impl IntoElement,
) -> Stateful<Div> {
    let id = id.into();
    let selector = id.clone();
    h_flex()
        .id(id)
        .debug_selector(move || selector.to_string())
        .w_full()
        .gap_2()
        .px_1()
        .py_0p5()
        .items_center()
        .child(div().flex_none().w(rems(1.25)).child(icon))
        .child(div().flex_1().min_w_0().truncate().child(title))
        .when(fit.0 > 0, |row| {
            row.child(div().flex_none().w(rems(9.)).truncate().child(workspace))
        })
        .when(fit.0 > 1, |row| {
            row.child(div().flex_none().w(rems(6.)).truncate().child(device))
        })
        .when(fit.0 > 3, |row| {
            row.child(div().flex_none().w(rems(9.)).truncate().child(branch))
        })
        .when(fit.0 > 2, |row| {
            row.child(div().flex_none().w(rems(6.)).child(changes))
        })
        .child(div().min_w_0().w(rems(12.)).truncate().child(state))
}

impl Condr {
    pub(super) fn render_tasks_list(
        &self,
        columns: &[(TasksColumn, Vec<TaskCard>)],
        todo: Option<Vec<(ConnectionKey, TaskRecord)>>,
        scope: TasksScope,
        prefs: TasksPrefs,
        fit: ListFit,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let header = list_row(
            fit,
            "tasks-list-header",
            div(),
            t!("tasks.list.title").into_owned(),
            t!("tasks.list.workspace").into_owned(),
            t!("tasks.list.branch").into_owned(),
            t!("tasks.list.device").into_owned(),
            div().child(t!("tasks.list.changes").into_owned()),
            div().child(t!("tasks.list.state").into_owned()),
        )
        .text_xs()
        .text_color(theme.muted_foreground);
        let mut rows = vec![header.into_any_element()];
        if let Some(todo) = todo {
            let folded = prefs.is_folded(TasksColumn::ToDo);
            rows.push(
                self.column_heading(TasksColumn::ToDo, todo.len(), &theme, cx)
                    .pt_2()
                    .child(
                        Icon::new(IconName::ChevronRight)
                            .size_3()
                            .when(!folded, |icon| icon.rotate(percentage(90. / 360.))),
                    )
                    .into_any_element(),
            );
            if !folded {
                rows.push(self.render_new_task_row(fit, cx));
                rows.extend(
                    todo.iter()
                        .map(|(key, task)| self.render_todo_row(*key, task, fit, cx)),
                );
                rows.push(self.render_archived(scope, cx));
            }
        }
        for (column, cards) in columns {
            let folded = prefs.is_folded(*column);
            rows.push(
                self.column_heading(*column, cards.len(), &theme, cx)
                    .pt_2()
                    .child(
                        Icon::new(IconName::ChevronRight)
                            .size_3()
                            .when(!folded, |icon| icon.rotate(percentage(90. / 360.))),
                    )
                    .into_any_element(),
            );
            if !folded {
                rows.extend(cards.iter().map(|card| match card.task {
                    Some(task) => self.render_task_row(card, task, fit, cx),
                    None => self.render_list_row(card, fit, cx),
                }));
            }
        }
        v_flex()
            .id("tasks-list")
            .debug_selector(|| "tasks-list".into())
            .size_full()
            .p_3()
            .gap_1()
            .overflow_y_scrollbar()
            .children(rows)
            .into_any_element()
    }
}
