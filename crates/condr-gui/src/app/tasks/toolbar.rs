//! Tasks mode's title and the toolbar over its cards: the scope's menu on the left,
//! Layout and the way back on the right (ADR 0068).

use super::*;
use gpui_kit::component::button::ButtonGroup;
use gpui_kit::component::popover::Popover;
use gpui_kit::component::separator::Separator;
use gpui_kit::component::switch::Switch;
use rust_i18n::t;

/// What a row of the scope's menu names, which sets its indent, icon and colour.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum ScopeRow {
    All,
    Device { local: bool },
    Workspace,
    Worktree,
}

/// A scope's row: indented by its level, its icon, its name, and its Agent count on the
/// right. All and the Devices lead in the primary colour; Workspaces and worktrees, and
/// counts, are secondary, a zero count fainter still.
pub(super) fn scope_row(
    row: ScopeRow,
    name: SharedString,
    count: usize,
    selected: bool,
    cx: &App,
) -> Div {
    let theme = cx.theme();
    let (indent, icon, primary) = match row {
        ScopeRow::All => (0., Icon::new(gpui_kit::assets::IconName::Layers), true),
        ScopeRow::Device { local: true } => {
            (0., Icon::new(gpui_kit::assets::IconName::Monitor), true)
        }
        ScopeRow::Device { local: false } => {
            (0., Icon::new(gpui_kit::assets::IconName::Server), true)
        }
        ScopeRow::Workspace => (1.25, Icon::new(IconName::Folder), false),
        ScopeRow::Worktree => (2.5, Icon::new(CondrIconName::GitBranch), false),
    };
    let color = if primary || selected {
        theme.foreground
    } else {
        theme.muted_foreground
    };
    h_flex()
        .flex_1()
        .min_w_0()
        .gap_2()
        .pl(rems(indent))
        .items_center()
        .text_color(color)
        .child(icon.small())
        .child(div().flex_1().min_w_0().truncate().child(name))
        .child(
            div()
                .flex_none()
                .pl_4()
                .text_xs()
                .text_color(theme.muted_foreground)
                .when(count == 0, |this| this.opacity(0.6))
                .child(count.to_string()),
        )
}

impl Condr {
    pub(super) fn set_tasks_layout(&mut self, layout: TasksLayout, cx: &mut Context<Self>) {
        self.main_view_mut().tasks.layout = layout;
        self.schedule_state_save(cx);
        cx.notify();
    }

    pub(super) fn set_tasks_density(&mut self, density: TasksDensity, cx: &mut Context<Self>) {
        self.main_view_mut().tasks.density = density;
        self.schedule_state_save(cx);
        cx.notify();
    }

    /// Tasks mode's title in the title bar, where the Tab strip was, as the Ports page's.
    pub(in crate::app) fn render_tasks_title(&self, cx: &mut Context<Self>) -> AnyElement {
        h_flex()
            .debug_selector(|| "tasks-title".into())
            .gap_2()
            .items_center()
            .px_2()
            .child(Icon::new(CondrIconName::SquareKanban).size_4())
            .child(div().text_sm().child(t!("tasks.title").into_owned()))
            .text_color(cx.theme().foreground)
            .into_any_element()
    }

    /// Over the cards and the interaction area: the scope on the left, Layout and the way
    /// back to the Workspace on the right, as the Ports page closes.
    pub(super) fn tasks_toolbar(
        &self,
        prefs: TasksPrefs,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let close_owner = cx.weak_entity();
        let back = t!("tasks.back");
        h_flex()
            .debug_selector(|| "tasks-toolbar".into())
            .flex_none()
            .w_full()
            .gap_2()
            .px_2()
            .py_1()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(self.render_scope_menu(prefs.scope, cx))
            .child(div().flex_1())
            .child(self.render_layout_popover(cx))
            .child(
                Button::new("tasks-close")
                    .debug_selector(|| "tasks-close".into())
                    .ghost()
                    .small()
                    .icon(IconName::Close)
                    .tooltip_with_action(back, &ToggleTasks, Some(SHORTCUT_CONTEXT))
                    .on_click(move |_, window, cx| {
                        let _ = close_owner.update(cx, |this, cx| this.toggle_tasks(window, cx));
                    }),
            )
    }

    /// Layout: List or Board, and whether the Board's cards are compact; the List's rows
    /// always are.
    pub(super) fn render_layout_popover(&self, cx: &mut Context<Self>) -> AnyElement {
        let owner = cx.weak_entity();
        Popover::new("tasks-layout-popover")
            .anchor(Anchor::TopRight)
            .trigger(
                Button::new("tasks-layout")
                    .debug_selector(|| "tasks-layout".into())
                    .ghost()
                    .small()
                    .icon(Icon::new(IconName::LayoutDashboard))
                    .label(t!("tasks.layout.button")),
            )
            .content(move |_, _, cx| {
                let prefs = owner
                    .upgrade()
                    .map(|owner| owner.read(cx).main_view().tasks)
                    .unwrap_or_default();
                let board = prefs.layout == TasksLayout::Board;
                let (layout_owner, density_owner) = (owner.clone(), owner.clone());
                let layout = ButtonGroup::new("tasks-layout-toggle")
                    .outline()
                    .small()
                    .w_full()
                    .child(
                        Button::new("tasks-layout-list")
                            .debug_selector(|| "tasks-layout-list".into())
                            .flex_1()
                            .icon(Icon::new(gpui_kit::assets::IconName::Table))
                            .label(t!("tasks.layout.list"))
                            .selected(!board),
                    )
                    .child(
                        Button::new("tasks-layout-board")
                            .debug_selector(|| "tasks-layout-board".into())
                            .flex_1()
                            .icon(Icon::new(gpui_kit::assets::IconName::Kanban))
                            .label(t!("tasks.layout.board"))
                            .selected(board),
                    )
                    .on_click(move |selected, _, cx| {
                        let layout = if selected.first() == Some(&1) {
                            TasksLayout::Board
                        } else {
                            TasksLayout::List
                        };
                        let _ =
                            layout_owner.update(cx, |this, cx| this.set_tasks_layout(layout, cx));
                    });
                let compact = Switch::new("tasks-compact")
                    .checked(prefs.density == TasksDensity::Compact)
                    .on_click(move |compact, _, cx| {
                        let density = if *compact {
                            TasksDensity::Compact
                        } else {
                            TasksDensity::Cards
                        };
                        let _ = density_owner
                            .update(cx, |this, cx| this.set_tasks_density(density, cx));
                    });
                let muted = cx.theme().muted_foreground;
                // The layout leads; its options sit under a rule in the secondary colour,
                // and the List, whose rows are always compact, has none.
                v_flex()
                    .w(rems(16.))
                    .gap_2()
                    .child(layout)
                    .when(board, |this| {
                        this.child(Separator::horizontal()).child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .text_sm()
                                .text_color(muted)
                                .child(Icon::new(gpui_kit::assets::IconName::Rows2).small())
                                .child(t!("tasks.compact_cards.label").into_owned())
                                .child(div().flex_1())
                                // Kit's unchecked track all but vanishes on a popover; the
                                // word says the state.
                                .child(div().text_xs().child(
                                    if prefs.density == TasksDensity::Compact {
                                        t!("tasks.compact_cards.on").into_owned()
                                    } else {
                                        t!("tasks.compact_cards.off").into_owned()
                                    },
                                ))
                                .child(
                                    div()
                                        .debug_selector(|| "tasks-compact".into())
                                        .child(compact),
                                ),
                        )
                    })
            })
            .into_any_element()
    }

    /// `MacBook › api-server ▾`: the scope as a breadcrumb that opens a grouped tree:
    /// All, then each Device over its Workspaces, worktrees under their repository, each
    /// row with its Agent count on the right.
    pub(super) fn render_scope_menu(
        &self,
        scope: TasksScope,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let scope = self.effective_tasks_scope(scope);
        let count = |scope| -> usize {
            self.task_cards(scope, None)
                .iter()
                .map(|(_, cards)| cards.len())
                .sum()
        };
        let mut crumbs = Vec::new();
        let mut options = vec![(
            ScopeRow::All,
            t!("tasks.scope.all").into_owned(),
            TasksScope::All,
        )];
        for connection in &self.connections {
            let Some(server) = connection.model.server_id else {
                continue;
            };
            options.push((
                ScopeRow::Device {
                    local: connection.endpoint.as_local_path().is_some(),
                },
                connection.label.clone(),
                TasksScope::Device(server),
            ));
            if scope == TasksScope::Device(server) {
                crumbs.push(connection.label.clone());
            }
            let Some(session) = connection.presented_session() else {
                continue;
            };
            let parent_of = |workspace: &Workspace| {
                workspace
                    .worktree()
                    .map(|association| association.parent_workspace_id())
                    .filter(|parent| session.workspace(*parent).is_some())
            };
            for repository in session
                .workspaces()
                .iter()
                .filter(|each| parent_of(each).is_none())
            {
                let repository_scope = TasksScope::Workspace(server, repository.id());
                options.push((
                    ScopeRow::Workspace,
                    repository.name().to_owned(),
                    repository_scope,
                ));
                if scope == repository_scope {
                    crumbs.extend([connection.label.clone(), repository.name().to_owned()]);
                }
                for worktree in session
                    .workspaces()
                    .iter()
                    .filter(|each| parent_of(each) == Some(repository.id()))
                {
                    let worktree_scope = TasksScope::Workspace(server, worktree.id());
                    options.push((
                        ScopeRow::Worktree,
                        worktree.name().to_owned(),
                        worktree_scope,
                    ));
                    if scope == worktree_scope {
                        crumbs.extend([
                            connection.label.clone(),
                            repository.name().to_owned(),
                            worktree.name().to_owned(),
                        ]);
                    }
                }
            }
        }
        if scope == TasksScope::All {
            crumbs.push(t!("tasks.scope.all").into_owned());
        }
        let options: Vec<(ScopeRow, SharedString, usize, TasksScope)> = options
            .into_iter()
            .map(|(row, name, each)| (row, name.into(), count(each), each))
            .collect();
        let owner = cx.weak_entity();
        Button::new("tasks-scope")
            .debug_selector(|| "tasks-scope".into())
            .ghost()
            .small()
            .icon(Icon::new(gpui_kit::assets::IconName::ListFilter))
            .label(crumbs.join(" › "))
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                let menu = menu.min_w(px(260.)).max_h(px(480.)).scrollable(true);
                options.iter().fold(menu, |menu, (row, name, count, each)| {
                    let (owner, each, row, count) = (owner.clone(), *each, *row, *count);
                    let name = name.clone();
                    // Each Device starts a group under a rule.
                    let menu = if matches!(row, ScopeRow::Device { .. }) {
                        menu.separator()
                    } else {
                        menu
                    };
                    menu.item(
                        PopupMenuItem::element(move |_, cx| {
                            scope_row(row, name.clone(), count, each == scope, cx)
                        })
                        .checked(each == scope)
                        .on_click(move |_, _, cx| {
                            let _ = owner.update(cx, |this, cx| this.set_tasks_scope(each, cx));
                        }),
                    )
                })
            })
            .into_any_element()
    }
}
