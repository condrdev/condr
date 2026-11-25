//! The sidebar's way into Tasks mode: its row at the top, and its icon heading the
//! collapsed sidebar's avatars (ADR 0068).

use super::*;
use rust_i18n::t;

impl Condr {
    /// The Tasks row at the top of the main window's sidebar, with To do's count.
    /// The Tasks row, lit while Tasks mode shows, as a Workspace's row is while it shows.
    pub(in crate::app) fn render_tasks_entry(
        &self,
        active: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let owner = cx.weak_entity();
        let todo = self.todo_count();
        let muted = cx.theme().muted_foreground;
        let row = h_flex()
            .id("sidebar-tasks")
            .debug_selector(|| "sidebar-tasks".into())
            .h_8()
            .w_full()
            .px_2()
            .gap_2()
            .items_center()
            .rounded(cx.theme().radius)
            .text_sm()
            .font_semibold()
            .map(|this| crate::app::list_row::row_highlight(this, active, cx))
            .child(Icon::new(CondrIconName::SquareKanban).size_4())
            .child(div().flex_1().child(t!("tasks.title").into_owned()))
            .when(todo > 0, |this| {
                this.child(
                    div()
                        .debug_selector(|| "sidebar-tasks-count".into())
                        .font_normal()
                        .text_color(muted)
                        .child(todo.to_string()),
                )
            })
            .on_click(move |_, window, cx| {
                let _ = owner.update(cx, |this, cx| this.toggle_tasks(window, cx));
            });
        v_flex()
            .flex_1()
            .gap_2()
            .child(row)
            .child(Separator::horizontal().color(cx.theme().sidebar_border))
            .into_any_element()
    }

    /// The Tasks icon heading the collapsed sidebar's avatars.
    pub(in crate::app) fn render_collapsed_tasks_entry(
        &self,
        active: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let owner = cx.weak_entity();
        Button::new("collapsed-tasks")
            .debug_selector(|| "collapsed-tasks".into())
            .ghost()
            .small()
            .icon(Icon::new(CondrIconName::SquareKanban))
            .selected(active)
            .tooltip_with_action(t!("tasks.title"), &ToggleTasks, Some(SHORTCUT_CONTEXT))
            .on_click(move |_, window, cx| {
                let _ = owner.update(cx, |this, cx| this.toggle_tasks(window, cx));
            })
            .into_any_element()
    }
}
